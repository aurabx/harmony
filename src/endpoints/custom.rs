use std::any::Any;
use crate::endpoints::EndpointHandler;

pub trait CustomEndpointFactory: Send + Sync {
    fn name(&self) -> &'static str;
    fn create_handler(&self) -> Box<dyn EndpointHandler>;
    fn as_any(&self) -> &dyn Any;
}
