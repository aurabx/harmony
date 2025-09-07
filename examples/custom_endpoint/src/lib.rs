
// Example custom endpoint implementation
pub struct MyCustomEndpointFactory;

impl CustomEndpointFactory for MyCustomEndpointFactory {
    fn name(&self) -> &'static str {
        "my_custom_endpoint"
    }

    fn create(&self) -> Box<dyn EndpointType> {
        Box::new(BasicEndpoint {
            path_prefix: Some("/custom".to_string()),
        })
    }
}