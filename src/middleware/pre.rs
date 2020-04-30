use crate::prelude::*;
use crate::regex_generator::generate_exact_match_regex;
use hyper::{body::HttpBody, Request};
use regex::Regex;
use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::pin::Pin;

type Handler<B, E> = Box<dyn FnMut(Request<B>) -> HandlerReturn<B, E> + Send + Sync + 'static>;
type HandlerReturn<B, E> = Box<dyn Future<Output = Result<Request<B>, E>> + Send + 'static>;

pub struct PreMiddleware<B, E> {
    pub(crate) path: String,
    regex: Regex,
    // Make it an option so that when a router is used to scope in another router,
    // It can be extracted out by 'opt.take()' without taking the whole router's ownership.
    pub(crate) handler: Option<Handler<B, E>>,
}

impl<B: HttpBody + Send + Sync + Unpin + 'static, E: std::error::Error + Send + Sync + Unpin + 'static>
    PreMiddleware<B, E>
{
    pub(crate) fn new_with_boxed_handler<P: Into<String>>(
        path: P,
        handler: Handler<B, E>,
    ) -> crate::Result<PreMiddleware<B, E>> {
        let path = path.into();
        let (re, _) = generate_exact_match_regex(path.as_str())
            .context("Could not create an exact match regex for the pre middleware path")?;

        Ok(PreMiddleware {
            path,
            regex: re,
            handler: Some(handler),
        })
    }

    pub fn new<P, H, R>(path: P, mut handler: H) -> crate::Result<PreMiddleware<B, E>>
    where
        P: Into<String>,
        H: FnMut(Request<B>) -> R + Send + Sync + 'static,
        R: Future<Output = Result<Request<B>, E>> + Send + 'static,
    {
        let handler: Handler<B, E> = Box::new(move |req: Request<B>| Box::new(handler(req)));
        PreMiddleware::new_with_boxed_handler(path, handler)
    }

    pub(crate) fn is_match(&self, target_path: &str) -> bool {
        self.regex.is_match(target_path)
    }

    pub(crate) async fn process(&mut self, req: Request<B>) -> crate::Result<Request<B>> {
        let handler = self
            .handler
            .as_mut()
            .expect("A router can not be used after mounting into another router");

        Pin::from(handler(req)).await.wrap()
    }
}

impl<B, E> Debug for PreMiddleware<B, E> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{{ path: {:?}, regex: {:?} }}", self.path, self.regex)
    }
}
