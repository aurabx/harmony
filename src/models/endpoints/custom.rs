use crate::models::endpoints::endpoint_type::EndpointType;

pub trait CustomEndpointFactory: Send + Sync {
    fn name(&self) -> &'static str;
    fn create(&self) -> Box<dyn EndpointType<ReqBody=(), ResBody=()>>;
}
