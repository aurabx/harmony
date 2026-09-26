//! The routes of one network and the rule that picks an endpoint for a request.
//!
//! Endpoints on a network may overlap. A collection endpoint at
//! `/v1/checkout/sessions` also serves everything below it, and an item
//! endpoint at `/v1/checkout/sessions/{id}/complete` sits inside that space.
//! axum's path matcher cannot hold a `{param}` and a catch-all at the same
//! position, so Harmony matches paths itself and gives axum one catch-all.
//!
//! # Matching rule
//!
//! A route pattern is a list of `/`-separated segments. Each segment is a
//! literal (`sessions`), a parameter (`{id}`) or a catch-all (`{*rest}`, last
//! segment only). A parameter or catch-all may have literal text in front of
//! it (`img_{id}`).
//!
//! 1. Keep the routes whose pattern matches the path. Matching is done by
//!    `matchit`, the matcher axum uses, so what a pattern matches is unchanged.
//!    No route matches: 404.
//! 2. Pick the most specific route. Compare two patterns segment by segment
//!    from the left. At the first segment where they differ, a literal beats
//!    a parameter and a parameter beats a catch-all. Between two parameters
//!    or two catch-alls, the one with more literal text in front wins.
//! 3. If that route allows the method, it serves the request. A route that
//!    allows GET also serves HEAD, as axum does. Otherwise the answer is 405
//!    with an `Allow` header listing that route's methods.
//!
//! The method never sends a request to a less specific route. Endpoints on
//! one network can belong to different pipelines, with different auth,
//! policies and backends. A client must not reach another pipeline by
//! changing the method.
//!
//! The route is chosen by path alone, so two routes with the same shape
//! (same segments, parameter names ignored) from different endpoints can
//! never be told apart, whatever their methods. That is a configuration
//! error, reported by [`RouteTable::build`] and turned into a config load
//! error by `Config::validate`. One endpoint may register the same shape
//! more than once with different methods; those routes are merged.

use crate::config::config::Config;
use crate::router::route_config::RouteConfig;
use http::Method;
use std::cmp::Ordering;
use std::fmt;

/// Kind of one pattern segment. The derived order is most specific first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum SegmentKind {
    Literal,
    Param,
    CatchAll,
}

/// One `/`-separated segment of a route pattern.
///
/// Two segments are equal when they have the same kind and the same literal
/// text. Parameter names are not kept, so `{id}` and `{sku}` are equal.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
    kind: SegmentKind,
    /// The whole segment for a literal; the text before `{` otherwise.
    literal: String,
}

impl Segment {
    /// Order two segments that matched the same part of a request path.
    /// `Less` means `self` is more specific.
    fn specificity_cmp(&self, other: &Segment) -> Ordering {
        self.kind
            .cmp(&other.kind)
            .then_with(|| other.literal.len().cmp(&self.literal.len()))
    }
}

/// Split a pattern that `matchit` already accepted into segments.
///
/// `{{` and `}}` are escaped braces and count as literal text.
fn parse_segments(pattern: &str) -> Vec<Segment> {
    pattern
        .strip_prefix('/')
        .unwrap_or(pattern)
        .split('/')
        .map(parse_segment)
        .collect()
}

fn parse_segment(text: &str) -> Segment {
    let mut literal = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                literal.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                literal.push('}');
            }
            '{' => {
                let kind = if chars.peek() == Some(&'*') {
                    SegmentKind::CatchAll
                } else {
                    SegmentKind::Param
                };
                return Segment { kind, literal };
            }
            _ => literal.push(c),
        }
    }
    Segment {
        kind: SegmentKind::Literal,
        literal,
    }
}

/// The endpoint and pipeline that registered a route, and the file that
/// defined the endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteOwner {
    pub endpoint: String,
    pub pipeline: String,
    /// Config file that defined the endpoint. `None` when the config was
    /// not loaded from files.
    pub source: Option<String>,
}

impl fmt::Display for RouteOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "endpoint '{}' (pipeline '{}', ",
            self.endpoint, self.pipeline
        )?;
        match &self.source {
            Some(source) => write!(f, "defined in {})", source),
            None => write!(f, "source file unknown)"),
        }
    }
}

/// A route that could not be added to a [`RouteTable`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteProblem {
    /// The endpoint produced a path that is not a valid route pattern.
    InvalidPattern {
        owner: RouteOwner,
        pattern: String,
        reason: String,
    },
    /// Two endpoints (or one endpoint in two pipelines) register routes with
    /// the same shape.
    Conflict {
        existing: RouteOwner,
        existing_pattern: String,
        rejected: RouteOwner,
        rejected_pattern: String,
    },
}

impl RouteProblem {
    /// The route owners involved: the endpoint with the bad path, or the two
    /// endpoints in conflict. An endpoint registers several routes, so one
    /// mistake can produce several problems with the same owners.
    pub fn owners(&self) -> (&RouteOwner, Option<&RouteOwner>) {
        match self {
            RouteProblem::InvalidPattern { owner, .. } => (owner, None),
            RouteProblem::Conflict {
                existing, rejected, ..
            } => (existing, Some(rejected)),
        }
    }
}

impl fmt::Display for RouteProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RouteProblem::InvalidPattern {
                owner,
                pattern,
                reason,
            } => write!(
                f,
                "{} has invalid route path '{}': {}",
                owner, pattern, reason
            ),
            RouteProblem::Conflict {
                existing,
                existing_pattern,
                rejected,
                rejected_pattern,
            } => {
                let paths = if existing_pattern == rejected_pattern {
                    format!("'{}'", existing_pattern)
                } else {
                    format!("'{}' and '{}'", existing_pattern, rejected_pattern)
                };
                write!(
                    f,
                    "route conflict: {} and {} both route {}. \
                     Endpoints are chosen by path, whatever their methods, so a request \
                     cannot be sent to one of them over the other. \
                     Give one endpoint a different path_prefix, or put both on different networks",
                    existing, rejected, paths
                )
            }
        }
    }
}

/// One route pattern with the methods, endpoint and pipeline it serves.
#[derive(Debug)]
pub struct RouteEntry {
    pub pattern: String,
    pub methods: Vec<Method>,
    pub owner: RouteOwner,
    segments: Vec<Segment>,
    matcher: matchit::Router<()>,
}

impl RouteEntry {
    /// Check `route.path` and build the entry. The error is the reason the
    /// path is not a valid pattern.
    fn new(route: RouteConfig, owner: RouteOwner) -> Result<Self, String> {
        if !route.path.starts_with('/') {
            return Err("a route path must start with '/'".to_string());
        }
        let mut matcher = matchit::Router::new();
        matcher
            .insert(route.path.as_str(), ())
            .map_err(|e| e.to_string())?;
        Ok(Self {
            segments: parse_segments(&route.path),
            pattern: route.path,
            methods: route.methods,
            owner,
            matcher,
        })
    }

    fn matches(&self, path: &str) -> bool {
        self.matcher.at(path).is_ok()
    }

    fn allows(&self, method: &Method) -> bool {
        self.methods.contains(method) || (*method == Method::HEAD && self.allows_get())
    }

    fn allows_get(&self) -> bool {
        self.methods.contains(&Method::GET)
    }

    /// `Less` means `self` is more specific. Only meaningful for two entries
    /// that match the same path.
    fn specificity_cmp(&self, other: &RouteEntry) -> Ordering {
        for (mine, theirs) in self.segments.iter().zip(&other.segments) {
            let order = mine.specificity_cmp(theirs);
            if order != Ordering::Equal {
                return order;
            }
        }
        // Two patterns that match the same path and agree on every shared
        // segment differ in length only in malformed cases; prefer the
        // longer pattern so the result stays deterministic.
        other.segments.len().cmp(&self.segments.len())
    }

    /// The methods for the `Allow` header: the route's methods, plus HEAD
    /// when it allows GET, in name order.
    fn allow_header_methods(&self) -> Vec<Method> {
        let mut allowed = self.methods.clone();
        if self.allows_get() && !allowed.contains(&Method::HEAD) {
            allowed.push(Method::HEAD);
        }
        allowed.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        allowed
    }
}

/// Result of looking up a request in a [`RouteTable`].
#[derive(Debug)]
pub enum RouteLookup<'a> {
    Found(&'a RouteEntry),
    /// The most specific route for the path does not allow the method. Holds
    /// that route's methods, for the `Allow` header.
    MethodNotAllowed(Vec<Method>),
    NotFound,
}

/// All HTTP routes on one network.
#[derive(Debug, Default)]
pub struct RouteTable {
    entries: Vec<RouteEntry>,
}

impl RouteTable {
    /// Collect the routes of every endpoint in every pipeline on `network`.
    ///
    /// Pipelines are read in name order so the result does not depend on
    /// hash map order. A route that is invalid or conflicts with an earlier
    /// route is left out and reported in the returned problems. Endpoints
    /// that are missing or whose service does not resolve are skipped and
    /// logged; config validation reports those separately.
    pub fn build(config: &Config, network: &str) -> (RouteTable, Vec<RouteProblem>) {
        let mut pipeline_names: Vec<&String> = config
            .pipelines
            .iter()
            .filter(|(_, pipeline)| pipeline.networks.iter().any(|n| n == network))
            .map(|(name, _)| name)
            .collect();
        pipeline_names.sort();

        let mut table = RouteTable::default();
        let mut problems = Vec::new();

        for pipeline_name in pipeline_names {
            let pipeline = &config.pipelines[pipeline_name];
            for endpoint_name in &pipeline.endpoints {
                let Some(endpoint) = config.endpoints.get(endpoint_name) else {
                    tracing::warn!(
                        "Endpoint '{}' of pipeline '{}' not found in config; no routes registered",
                        endpoint_name,
                        pipeline_name
                    );
                    continue;
                };
                let service = match endpoint.resolve_service() {
                    Ok(service) => service,
                    Err(err) => {
                        tracing::error!(
                            "Failed to resolve service for endpoint '{}': {}",
                            endpoint_name,
                            err
                        );
                        continue;
                    }
                };
                let options = endpoint.options.clone().unwrap_or_default();
                let owner = RouteOwner {
                    endpoint: endpoint_name.clone(),
                    pipeline: pipeline_name.clone(),
                    source: config.endpoint_sources.get(endpoint_name).cloned(),
                };
                for route in service.build_router(&options) {
                    if let Err(problem) = table.add(route, owner.clone()) {
                        problems.push(*problem);
                    }
                }
            }
        }

        (table, problems)
    }

    /// Add one route unless it is invalid or has the same shape as another
    /// owner's route already in the table. A route with the same shape and
    /// the same owner adds its methods to the existing entry.
    fn add(&mut self, route: RouteConfig, owner: RouteOwner) -> Result<(), Box<RouteProblem>> {
        let pattern = route.path.clone();
        let entry = RouteEntry::new(route, owner.clone()).map_err(|reason| {
            Box::new(RouteProblem::InvalidPattern {
                owner,
                pattern,
                reason,
            })
        })?;

        let Some(existing) = self
            .entries
            .iter_mut()
            .find(|existing| existing.segments == entry.segments)
        else {
            self.entries.push(entry);
            return Ok(());
        };

        if existing.owner != entry.owner {
            return Err(Box::new(RouteProblem::Conflict {
                existing: existing.owner.clone(),
                existing_pattern: existing.pattern.clone(),
                rejected: entry.owner,
                rejected_pattern: entry.pattern,
            }));
        }
        for method in entry.methods {
            if !existing.methods.contains(&method) {
                existing.methods.push(method);
            }
        }
        Ok(())
    }

    pub fn entries(&self) -> &[RouteEntry] {
        &self.entries
    }

    /// Find the route that serves `method` on `path`, following the rule in
    /// the module documentation.
    pub fn lookup(&self, method: &Method, path: &str) -> RouteLookup<'_> {
        let Some(entry) = self.most_specific_match(path) else {
            return RouteLookup::NotFound;
        };
        if entry.allows(method) {
            RouteLookup::Found(entry)
        } else {
            RouteLookup::MethodNotAllowed(entry.allow_header_methods())
        }
    }

    /// The most specific entry whose pattern matches `path`, whatever its
    /// methods.
    ///
    /// Equal specificity means the same shape, and `add` keeps one entry per
    /// shape, so there are no ties between matching entries. `min_by` keeps
    /// the entry added first if that ever changes.
    fn most_specific_match(&self, path: &str) -> Option<&RouteEntry> {
        self.entries
            .iter()
            .filter(|e| e.matches(path))
            .min_by(|a, b| a.specificity_cmp(b))
    }
}

/// Methods joined for messages and the `Allow` header, e.g. `GET, POST`.
pub fn method_list(methods: &[Method]) -> String {
    methods
        .iter()
        .map(Method::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(endpoint: &str) -> RouteOwner {
        RouteOwner {
            endpoint: endpoint.to_string(),
            pipeline: format!("{}_pipeline", endpoint),
            source: None,
        }
    }

    fn route(path: &str, methods: &[Method]) -> RouteConfig {
        RouteConfig {
            path: path.to_string(),
            methods: methods.to_vec(),
            description: None,
        }
    }

    /// The two routes an `http` endpoint registers for `prefix`.
    fn add_prefix(table: &mut RouteTable, endpoint: &str, prefix: &str, methods: &[Method]) {
        table
            .add(route(prefix, methods), owner(endpoint))
            .expect("exact route");
        table
            .add(
                route(&format!("{}/{{*wildcard}}", prefix), methods),
                owner(endpoint),
            )
            .expect("wildcard route");
    }

    fn endpoint_for(table: &RouteTable, method: Method, path: &str) -> Option<String> {
        match table.lookup(&method, path) {
            RouteLookup::Found(entry) => Some(entry.owner.endpoint.clone()),
            _ => None,
        }
    }

    const ALL: &[Method] = &[
        Method::GET,
        Method::HEAD,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
    ];

    fn ucp_table() -> RouteTable {
        let mut table = RouteTable::default();
        add_prefix(&mut table, "sessions", "/v1/checkout/sessions", ALL);
        add_prefix(&mut table, "session", "/v1/checkout/sessions/{id}", ALL);
        add_prefix(
            &mut table,
            "complete",
            "/v1/checkout/sessions/{id}/complete",
            ALL,
        );
        table
    }

    #[test]
    fn ucp_checkout_shape_routes_to_the_most_specific_endpoint() {
        let table = ucp_table();
        let cases = [
            (Method::POST, "/v1/checkout/sessions", "sessions"),
            (Method::GET, "/v1/checkout/sessions", "sessions"),
            (Method::GET, "/v1/checkout/sessions/cs_1", "session"),
            (Method::PATCH, "/v1/checkout/sessions/cs_1", "session"),
            (
                Method::POST,
                "/v1/checkout/sessions/cs_1/complete",
                "complete",
            ),
            (
                Method::POST,
                "/v1/checkout/sessions/cs_1/complete/x",
                "complete",
            ),
            (Method::GET, "/v1/checkout/sessions/cs_1/lines", "session"),
        ];
        for (method, path, expected) in cases {
            assert_eq!(
                endpoint_for(&table, method.clone(), path).as_deref(),
                Some(expected),
                "{} {}",
                method,
                path
            );
        }
        assert!(matches!(
            table.lookup(&Method::GET, "/v1/other"),
            RouteLookup::NotFound
        ));
    }

    #[test]
    fn literal_beats_param_beats_catch_all() {
        let mut table = RouteTable::default();
        table
            .add(route("/a/{*rest}", ALL), owner("catch_all"))
            .unwrap();
        table.add(route("/a/{id}", ALL), owner("param")).unwrap();
        table.add(route("/a/new", ALL), owner("literal")).unwrap();

        assert_eq!(
            endpoint_for(&table, Method::GET, "/a/new").as_deref(),
            Some("literal")
        );
        assert_eq!(
            endpoint_for(&table, Method::GET, "/a/7").as_deref(),
            Some("param")
        );
        assert_eq!(
            endpoint_for(&table, Method::GET, "/a/7/b").as_deref(),
            Some("catch_all")
        );
    }

    #[test]
    fn leftmost_difference_decides() {
        let mut table = RouteTable::default();
        table
            .add(route("/a/{x}/c", ALL), owner("late_literal"))
            .unwrap();
        table
            .add(route("/a/b/{y}", ALL), owner("early_literal"))
            .unwrap();

        assert_eq!(
            endpoint_for(&table, Method::GET, "/a/b/c").as_deref(),
            Some("early_literal")
        );
        assert_eq!(
            endpoint_for(&table, Method::GET, "/a/z/c").as_deref(),
            Some("late_literal")
        );
    }

    #[test]
    fn literal_text_before_a_parameter_is_more_specific() {
        let mut table = RouteTable::default();
        table
            .add(route("/files/{name}", ALL), owner("plain"))
            .unwrap();
        table
            .add(route("/files/img_{name}", ALL), owner("prefixed"))
            .unwrap();

        assert_eq!(
            endpoint_for(&table, Method::GET, "/files/img_1").as_deref(),
            Some("prefixed")
        );
        assert_eq!(
            endpoint_for(&table, Method::GET, "/files/doc").as_deref(),
            Some("plain")
        );
    }

    fn allowed_for(table: &RouteTable, method: Method, path: &str) -> String {
        match table.lookup(&method, path) {
            RouteLookup::MethodNotAllowed(allowed) => method_list(&allowed),
            other => panic!("{} {}: expected 405, got {:?}", method, path, other),
        }
    }

    #[test]
    fn a_method_the_most_specific_route_lacks_is_405_not_a_less_specific_route() {
        let mut table = RouteTable::default();
        add_prefix(&mut table, "proxy", "/api", ALL);
        table
            .add(route("/api/jmix/{id}", &[Method::GET]), owner("jmix"))
            .unwrap();

        assert_eq!(
            endpoint_for(&table, Method::GET, "/api/jmix/1").as_deref(),
            Some("jmix")
        );
        assert_eq!(
            allowed_for(&table, Method::DELETE, "/api/jmix/1"),
            "GET, HEAD"
        );
        // Paths the jmix route does not match still reach the proxy.
        assert_eq!(
            endpoint_for(&table, Method::DELETE, "/api/jmix/1/x").as_deref(),
            Some("proxy")
        );
    }

    #[test]
    fn head_goes_where_get_goes() {
        let mut table = RouteTable::default();
        table
            .add(route("/a", &[Method::GET]), owner("get_only"))
            .unwrap();
        table
            .add(route("/b/{id}", &[Method::GET]), owner("b_get"))
            .unwrap();
        table
            .add(route("/b/{*rest}", &[Method::HEAD]), owner("b_head"))
            .unwrap();

        assert_eq!(
            endpoint_for(&table, Method::HEAD, "/a").as_deref(),
            Some("get_only")
        );
        // The more specific GET route serves HEAD.
        assert_eq!(
            endpoint_for(&table, Method::HEAD, "/b/x").as_deref(),
            Some("b_get")
        );
        assert_eq!(
            endpoint_for(&table, Method::HEAD, "/b/x/y").as_deref(),
            Some("b_head")
        );
        // The less specific HEAD route never serves GET.
        assert_eq!(allowed_for(&table, Method::GET, "/b/x/y"), "HEAD");
    }

    #[test]
    fn method_not_allowed_lists_the_allowed_methods() {
        let mut table = RouteTable::default();
        table
            .add(route("/a", &[Method::POST, Method::GET]), owner("a"))
            .unwrap();

        match table.lookup(&Method::DELETE, "/a") {
            RouteLookup::MethodNotAllowed(allowed) => {
                assert_eq!(method_list(&allowed), "GET, HEAD, POST");
            }
            other => panic!("expected 405, got {:?}", other),
        }
        assert!(matches!(
            table.lookup(&Method::DELETE, "/b"),
            RouteLookup::NotFound
        ));
    }

    #[test]
    fn same_shape_with_shared_methods_conflicts() {
        let mut table = RouteTable::default();
        table
            .add(route("/items/{id}", ALL), owner("by_id"))
            .unwrap();

        let problem = table
            .add(route("/items/{sku}", &[Method::GET]), owner("by_sku"))
            .expect_err("conflict");
        match problem.as_ref() {
            RouteProblem::Conflict {
                existing, rejected, ..
            } => {
                assert_eq!(existing.endpoint, "by_id");
                assert_eq!(rejected.endpoint, "by_sku");
            }
            other => panic!("expected conflict, got {:?}", other),
        }
        let text = problem.to_string();
        assert!(
            text.contains("'/items/{id}' and '/items/{sku}'"),
            "{}",
            text
        );
        assert!(text.contains("source file unknown"), "{}", text);
    }

    #[test]
    fn same_shape_with_disjoint_methods_conflicts() {
        let mut table = RouteTable::default();
        table
            .add(route("/api/jmix", &[Method::GET]), owner("query"))
            .unwrap();

        let problem = table
            .add(route("/api/jmix", &[Method::POST]), owner("upload"))
            .expect_err("conflict");
        let (existing, rejected) = problem.owners();
        assert_eq!(existing.endpoint, "query");
        assert_eq!(rejected.map(|o| o.endpoint.as_str()), Some("upload"));
        assert_eq!(table.entries().len(), 1);
    }

    #[test]
    fn one_endpoint_may_split_methods_across_routes_of_the_same_shape() {
        // The jmix service registers GET and POST on `/api/jmix` as two routes.
        let mut table = RouteTable::default();
        table
            .add(
                route("/api/jmix", &[Method::GET, Method::HEAD]),
                owner("jmix"),
            )
            .unwrap();
        table
            .add(route("/api/jmix", &[Method::POST]), owner("jmix"))
            .unwrap();

        assert_eq!(table.entries().len(), 1);
        assert_eq!(
            endpoint_for(&table, Method::GET, "/api/jmix").as_deref(),
            Some("jmix")
        );
        assert_eq!(
            endpoint_for(&table, Method::POST, "/api/jmix").as_deref(),
            Some("jmix")
        );
        assert_eq!(
            allowed_for(&table, Method::DELETE, "/api/jmix"),
            "GET, HEAD, POST"
        );
    }

    #[test]
    fn one_endpoint_in_two_pipelines_conflicts() {
        let mut table = RouteTable::default();
        let first = owner("shared");
        let second = RouteOwner {
            pipeline: "other_pipeline".to_string(),
            ..owner("shared")
        };
        table.add(route("/a", &[Method::GET]), first).unwrap();
        assert!(table.add(route("/a", &[Method::POST]), second).is_err());
    }

    #[test]
    fn overlapping_prefixes_do_not_conflict() {
        // The shape from the AURA-2443 report, which made axum panic.
        let mut table = RouteTable::default();
        add_prefix(
            &mut table,
            "complete",
            "/v1/checkout/sessions/{id}/complete",
            ALL,
        );
        add_prefix(&mut table, "sessions", "/v1/checkout/sessions", ALL);
        assert_eq!(table.entries().len(), 4);
    }

    #[test]
    fn escaped_braces_are_literal_text() {
        let mut table = RouteTable::default();
        table.add(route("/a/{{x}}", ALL), owner("braces")).unwrap();
        table.add(route("/a/{x}", ALL), owner("param")).unwrap();

        assert_eq!(
            endpoint_for(&table, Method::GET, "/a/{x}").as_deref(),
            Some("braces")
        );
        assert_eq!(
            endpoint_for(&table, Method::GET, "/a/y").as_deref(),
            Some("param")
        );
    }

    #[test]
    fn invalid_patterns_are_reported_not_panicked() {
        let mut table = RouteTable::default();
        for bad in ["v1/orders", "", "/v1/{id}.json", "/a/{*rest}/b", "/a/{}"] {
            match table.add(route(bad, ALL), owner("bad")) {
                Err(problem) => match *problem {
                    RouteProblem::InvalidPattern { pattern, .. } => assert_eq!(pattern, bad),
                    other => panic!("{:?} should be invalid, got {:?}", bad, other),
                },
                other => panic!("{:?} should be invalid, got {:?}", bad, other),
            }
        }
        assert!(table.entries().is_empty());
    }

    #[test]
    fn segments_ignore_parameter_names() {
        assert_eq!(
            parse_segments("/a/{id}/{*rest}"),
            parse_segments("/a/{x}/{*tail}")
        );
        assert_ne!(parse_segments("/a/{id}"), parse_segments("/a/{*id}"));
        assert_ne!(parse_segments("/a/b"), parse_segments("/a/{b}"));
        assert_eq!(
            parse_segments("/img_{id}"),
            vec![Segment {
                kind: SegmentKind::Param,
                literal: "img_".to_string()
            }]
        );
    }
}
