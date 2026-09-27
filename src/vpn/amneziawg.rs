use super::common::{InterfaceStatus, ProviderConfig, query_provider};

pub async fn status(config: &ProviderConfig) -> InterfaceStatus {
    query_provider(config).await
}
