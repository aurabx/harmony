Minimum Requirements
1. HTTP Server

Accept incoming HTTP requests
Support methods: GET, POST, PUT, DELETE

2. Request Handling

Capture the full incoming path (everything after base URL)
Capture all incoming headers
Capture request body (if present)
Capture cookies
Capture caching

3. Upstream Forwarding

Configure upstream FHIR server URL
Forward request to: {upstream_url}{captured_path}
Forward all headers from incoming request
Forward request body (if present)
Use same HTTP method as incoming request

4. Response Handling

Capture upstream response status code
Capture all upstream response headers
Capture upstream response body
Return all of the above to client unchanged

5. Error Handling

Handle upstream server unreachable/timeout
Return appropriate HTTP error to client

That's it. No FHIR-specific logic needed since you're just proxying.
Optional but recommended:

Logging (request/response for debugging)
Timeout configuration
Request size limits