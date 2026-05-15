pub mod http_simple;

#[cfg(feature = "headless")]
pub mod headless;

#[cfg(feature = "tls-impersonation")]
pub mod tls_impersonation;

pub use http_simple::HttpSimple;

#[cfg(feature = "headless")]
pub use headless::HeadlessBrowser;

#[cfg(feature = "tls-impersonation")]
pub use tls_impersonation::TlsImpersonation;
