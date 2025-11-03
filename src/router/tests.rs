#[cfg(test)]
mod tests {
    use super::super::route_config::RouteConfig;
    use http::Method;

    #[test]
    fn test_route_config_creation() {
        let route = RouteConfig {
            path: "/api/test".to_string(),
            methods: vec![Method::GET, Method::POST],
            description: Some("Test route".to_string()),
        };

        assert_eq!(route.path, "/api/test");
        assert_eq!(route.methods.len(), 2);
        assert!(route.description.is_some());
        assert_eq!(route.description.unwrap(), "Test route");
    }

    #[test]
    fn test_route_config_no_description() {
        let route = RouteConfig {
            path: "/api/simple".to_string(),
            methods: vec![Method::GET],
            description: None,
        };

        assert_eq!(route.path, "/api/simple");
        assert_eq!(route.methods.len(), 1);
        assert!(route.description.is_none());
    }

    #[test]
    fn test_route_config_multiple_methods() {
        let route = RouteConfig {
            path: "/api/resource".to_string(),
            methods: vec![Method::GET, Method::POST, Method::PUT, Method::DELETE],
            description: Some("CRUD endpoint".to_string()),
        };

        assert_eq!(route.methods.len(), 4);
        assert!(route.methods.contains(&Method::GET));
        assert!(route.methods.contains(&Method::POST));
        assert!(route.methods.contains(&Method::PUT));
        assert!(route.methods.contains(&Method::DELETE));
    }

    #[test]
    fn test_route_config_empty_methods() {
        let route = RouteConfig {
            path: "/api/noop".to_string(),
            methods: vec![],
            description: None,
        };

        assert_eq!(route.methods.len(), 0);
    }

    #[test]
    fn test_route_config_clone() {
        let route = RouteConfig {
            path: "/api/test".to_string(),
            methods: vec![Method::GET],
            description: Some("Original".to_string()),
        };

        let cloned = route.clone();

        assert_eq!(route.path, cloned.path);
        assert_eq!(route.methods.len(), cloned.methods.len());
        assert_eq!(route.description, cloned.description);
    }

    #[test]
    fn test_route_config_debug_format() {
        let route = RouteConfig {
            path: "/api/debug".to_string(),
            methods: vec![Method::GET],
            description: Some("Debug test".to_string()),
        };

        let debug_str = format!("{:?}", route);
        assert!(debug_str.contains("/api/debug"));
        assert!(debug_str.contains("GET"));
    }

    #[test]
    fn test_route_config_path_patterns() {
        let routes = vec![
            RouteConfig {
                path: "/".to_string(),
                methods: vec![Method::GET],
                description: Some("Root".to_string()),
            },
            RouteConfig {
                path: "/api/v1/users/:id".to_string(),
                methods: vec![Method::GET],
                description: Some("User by ID".to_string()),
            },
            RouteConfig {
                path: "/api/v1/users/*".to_string(),
                methods: vec![Method::GET],
                description: Some("Wildcard match".to_string()),
            },
        ];

        assert_eq!(routes[0].path, "/");
        assert!(routes[1].path.contains(":id"));
        assert!(routes[2].path.contains("*"));
    }

    #[test]
    fn test_route_config_method_immutability() {
        let route = RouteConfig {
            path: "/api/test".to_string(),
            methods: vec![Method::GET],
            description: None,
        };

        // Verify we can access methods but the original route isn't mutated
        let methods = &route.methods;
        assert_eq!(methods.len(), 1);
        assert_eq!(route.methods.len(), 1);
    }

    #[test]
    fn test_route_config_all_http_methods() {
        let all_methods = vec![
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::HEAD,
            Method::OPTIONS,
            Method::PATCH,
        ];

        let route = RouteConfig {
            path: "/api/all".to_string(),
            methods: all_methods.clone(),
            description: Some("All methods".to_string()),
        };

        assert_eq!(route.methods.len(), 7);
        for method in all_methods {
            assert!(route.methods.contains(&method));
        }
    }
}
