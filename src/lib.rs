pub use self::error::Error;
pub(crate) use self::error::{ErrorExt, ResultExt};
pub use self::ext::RequestExt;
pub use self::helpers::{handle_request, handle_request_err};
pub use self::middleware::{Middleware, PostMiddleware, PreMiddleware};
pub use self::route::Route;
pub use self::router::{Router, RouterBuilder};
pub use self::types::{PathParams, RequestData};

mod error;
mod ext;
mod helpers;
mod middleware;
pub mod prelude;
mod route;
mod router;
mod types;

pub type Result<T> = std::result::Result<T, Error>;
