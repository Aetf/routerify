use crate::helpers;
use crate::middleware::{PostMiddleware, PreMiddleware};
use crate::prelude::*;
use crate::route::Route;
use hyper::{body::HttpBody, Request, Response};
use regex::RegexSet;
use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::pin::Pin;

pub use self::builder::RouterBuilder;

mod builder;

pub(crate) type ErrHandler<B> = Box<dyn FnMut(crate::Error) -> ErrHandlerReturn<B> + Send + Sync + 'static>;
pub(crate) type ErrHandlerReturn<B> = Box<dyn Future<Output = Response<B>> + Send + 'static>;

/// Represents a modular, lightweight and mountable router type.
///
/// A router consists of some routes, some pre-middlewares and some post-middlewares.
///
/// This `Router<B, E>` type accepts two type parameters: `B` and `E`.
///
/// * The `B` represents the response body type which will be used by route handlers and the middlewares and this body type must implement
///   the [HttpBody](https://docs.rs/hyper/0.13.5/hyper/body/trait.HttpBody.html) trait. For an instance, `B` could be [hyper::Body](https://docs.rs/hyper/0.13.5/hyper/body/struct.Body.html)
///   type.
/// * The `E` represents any error type which will be used by route handlers and the middlewares. This error type must implement the [std::error::Error](https://doc.rust-lang.org/std/error/trait.Error.html).
///
/// A `Router` can be created using the `Router::builder()` method.
///
/// # Examples
///
/// ```
/// use routerify::Router;
/// use hyper::{Response, Request, Body};
///
/// // A handler for "/about" page.
/// // We will use hyper::Body as response body type and hyper::Error as error type.
/// async fn about_handler(_: Request<Body>) -> Result<Response<Body>, hyper::Error> {
///     Ok(Response::new(Body::from("About page")))
/// }
///
/// # fn run() -> Router<Body, hyper::Error> {
/// // Create a router with hyper::Body as response body type and hyper::Error as error type.
/// let router: Router<Body, hyper::Error> = Router::builder()
///     .get("/about", about_handler)
///     .build()
///     .unwrap();
/// # router
/// # }
/// # run();
/// ```
pub struct Router<B, E> {
    pub(crate) pre_middlewares: Vec<PreMiddleware<E>>,
    pub(crate) routes: Vec<Route<B, E>>,
    pub(crate) post_middlewares: Vec<PostMiddleware<B, E>>,
    // This handler should be added only on root Router.
    // Any error handler attached to scoped router will be ignored.
    pub(crate) err_handler: Option<ErrHandler<B>>,

    // We'll initialize it from the RouterService via Router::init_regex_set() method.
    regex_set: Option<RegexSet>,
}

impl<B: HttpBody + Send + Sync + Unpin + 'static, E: std::error::Error + Send + Sync + Unpin + 'static> Router<B, E> {
    pub(crate) fn new(
        pre_middlewares: Vec<PreMiddleware<E>>,
        routes: Vec<Route<B, E>>,
        post_middlewares: Vec<PostMiddleware<B, E>>,
        err_handler: Option<ErrHandler<B>>,
    ) -> Self {
        Router {
            pre_middlewares,
            routes,
            post_middlewares,
            err_handler,
            regex_set: None,
        }
    }

    pub(crate) fn init_regex_set(&mut self) -> crate::Result<()> {
        let regex_iter = self
            .pre_middlewares
            .iter()
            .map(|m| m.regex.as_str())
            .chain(self.routes.iter().map(|r| r.regex.as_str()))
            .chain(self.post_middlewares.iter().map(|m| m.regex.as_str()));

        self.regex_set = Some(RegexSet::new(regex_iter).context("Couldn't create router RegexSet")?);

        Ok(())
    }

    /// Return a [RouterBuilder](./struct.RouterBuilder.html) instance to build a `Router`.
    pub fn builder() -> RouterBuilder<B, E> {
        builder::RouterBuilder::new()
    }

    pub(crate) async fn process(&mut self, req: Request<hyper::Body>) -> crate::Result<Response<B>> {
        let target_path =
            helpers::percent_decode_request_path(req.uri().path()).context("Couldn't percent decode request path")?;

        let (matched_pre_middleware_idxs, matched_route_idxs, matched_post_middleware_idxs) =
            self.match_regex_set(target_path.as_str());

        let mut transformed_req = req;
        for idx in matched_pre_middleware_idxs {
            let pre_middleware = &mut self.pre_middlewares[idx];

            transformed_req = pre_middleware
                .process(transformed_req)
                .await
                .context("One of the pre middlewares couldn't process the request")?;
        }

        let mut resp: Option<Response<B>> = None;
        for idx in matched_route_idxs {
            let route = &mut self.routes[idx];

            if route.is_match_method(transformed_req.method()) {
                let route_resp_res = route
                    .process(target_path.as_str(), transformed_req)
                    .await
                    .context("One of the routes couldn't process the request");

                let route_resp = match route_resp_res {
                    Ok(route_resp) => route_resp,
                    Err(err) => {
                        if let Some(ref mut err_handler) = self.err_handler {
                            Pin::from(err_handler(err)).await
                        } else {
                            return crate::Result::Err(err);
                        }
                    }
                };

                resp = Some(route_resp);
                break;
            }
        }

        if let None = resp {
            return Err(crate::Error::new("No handlers added to handle non-existent routes. Tips: Please add an '.any' route at the bottom to handle any routes."));
        }

        let mut transformed_res = resp.unwrap();
        for idx in matched_post_middleware_idxs {
            let post_middleware = &mut self.post_middlewares[idx];

            transformed_res = post_middleware
                .process(transformed_res)
                .await
                .context("One of the post middlewares couldn't process the response")?;
        }

        Ok(transformed_res)
    }

    fn match_regex_set(&self, target_path: &str) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
        let matches = self
            .regex_set
            .as_ref()
            .expect("The 'regex_set' field in Router is not initialized")
            .matches(target_path)
            .into_iter();

        let pre_middlewares_len = self.pre_middlewares.len();
        let routes_len = self.routes.len();
        let post_middlewares_len = self.post_middlewares.len();

        let mut matched_pre_middleware_idxs = Vec::new();
        let mut matched_route_idxs = Vec::new();
        let mut matched_post_middleware_idxs = Vec::new();

        for idx in matches {
            if idx < pre_middlewares_len {
                matched_pre_middleware_idxs.push(idx);
            } else if idx >= pre_middlewares_len && idx < (pre_middlewares_len + routes_len) {
                matched_route_idxs.push(idx - pre_middlewares_len);
            } else if idx >= (pre_middlewares_len + routes_len)
                && idx < (pre_middlewares_len + routes_len + post_middlewares_len)
            {
                matched_post_middleware_idxs.push(idx - pre_middlewares_len - routes_len);
            }
        }

        (
            matched_pre_middleware_idxs,
            matched_route_idxs,
            matched_post_middleware_idxs,
        )
    }
}

impl<B, E> Debug for Router<B, E> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{ Pre-Middlewares: {:?}, Routes: {:?}, Post-Middlewares: {:?}, ErrHandler: {:?} }}",
            self.pre_middlewares,
            self.routes,
            self.post_middlewares,
            self.err_handler.is_some()
        )
    }
}
