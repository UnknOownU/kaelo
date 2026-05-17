use crate::types::{FetchError, FetchRequest, FetchResponse, Strategy};

pub trait FetchBackend: Send + Sync {
    fn fetch(
        &self,
        request: FetchRequest,
    ) -> impl std::future::Future<Output = Result<FetchResponse, FetchError>> + Send;

    fn name(&self) -> Strategy;

    fn supports_probe(&self) -> bool {
        false
    }
}
