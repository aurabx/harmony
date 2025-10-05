use crate::models::services::services::ServiceType;

pub trait CustomEndpointFactory: Send + Sync {
    fn name(&self) -> &'static str;
fn create(&self) -> Box<dyn ServiceType<ReqBody=()>>;
}
