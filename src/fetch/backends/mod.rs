pub mod http_simple;
pub mod public_api;

#[cfg(feature = "headless")]
pub mod browser_pool;

#[cfg(feature = "headless")]
pub mod headless;

#[cfg(feature = "tls-impersonation")]
pub mod tls_impersonation;

pub use http_simple::HttpSimple;
pub use public_api::PublicApiBackend;

#[cfg(feature = "headless")]
pub use browser_pool::BrowserPool;

#[cfg(feature = "headless")]
pub use headless::HeadlessBrowser;

#[cfg(feature = "headless")]
pub use headless::shutdown_browser_pool;

#[cfg(feature = "tls-impersonation")]
pub use tls_impersonation::TlsImpersonation;
