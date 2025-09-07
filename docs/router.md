# Router Description

The router is responsible for routing incoming and outgoing requests through a structured series of layers, ensuring proper authentication, transformation, and communication with relevant services and backends. This document outlines the expected behavior of the router.

## Routing Structure

The router now follows the revised flow:

1. **Endpoint**
   - Handles external requests and responses from clients.
2. **Endpoint Middleware** (e.g., **Auth Check**)
   - Applies endpoint-specific authentication or other initial transformations.
3. **Service** (e.g., **FHIR**)
   - Processes requests and responses using internal services.
4. **Service Middleware** (e.g., **Transforms**)
   - Applies reusable service-level transformation logic between different stages.
5. **Service** (e.g., **DICOM**)
   - May pass the request to another service within the internal architecture.
6. **Backend Middleware** (e.g., **Auth Set**)
   - Adds backend-specific parameters or authentication logic for outgoing communication.
7. **Backend**
   - Communicates with external third-party services or systems.

## Behavior

### Incoming Requests

1. The request is received by the router for a specified path.
2. The router identifies the **Endpoint** based on the configured `path_prefix`.
3. The router selects the first matching group (i.e. a group with that endpoint - we might later define filters to further refine this)
4. The router pulls the middleware from the group and iterates over the configured services and middleware
   a. **Endpoint Middleware** applies any configured authentication or request modification.
   b. **Service Middleware** applies any necessary modifications, such as transformations or logging.
   c. Additional processing occurs via other internal **Services**, if required (e.g., **DICOM** calling another **DICOM** service).
   d. **Backend Middleware** for setting authentication parameters or request transformations.
5. Finally, the response is routed to the appropriate **Backend**
6. The **Backend** communicates with the external system and returns the response.
7. The response follows the reverse path. 
8. The final response is sent to the client.

### Outgoing Requests

1. The outgoing request is initiated by the application for a specific backend.
2. **Backend Middleware** applies any required modifications, such as setting authentication headers or custom parameters.
3. The router sends the request to the backend to interact with the external system.
4. **Service Middleware** processes any incoming data from the external backend.
5. The backend may relay the data to internal services (e.g., transforming **DICOM** data to **FHIR**).
6. Response transformations occur in the same order as incoming requests:
7. The final response is passed back to the originating service.

## Implementation Notes

- **Middleware Chains**: Middleware (both incoming and outgoing) is now applied in a strict sequence as outlined above.
- **Dynamic Routing**: Routes are dynamically configured based on the `network_name` and validated at startup.
- **Error Handling**: Enhanced error handling ensures missing middleware/backend configurations result in appropriate HTTP-level errors, such as `NOT_FOUND` or `BAD_REQUEST`.


Endpoint 
   -> Endpoint Middleware (eg Auth check)
   -> Service (eg FHIR) 
   -> Service Middleware (eg Transforms) 
   -> Service (eg DICOM)
   -> Backend Middleware (eg Auth set)
   -> Backend


1. : Handles routes, passes the request to the correct . **Router**`Endpoint`
2. :
   - Converts request to an . `Envelope`
   - Sends it through the middleware chain to the backend.

**Endpoint**
3. **Middleware (Request)**: Transforms and forwards to backend. `Envelope`
4. :
   - Processes the , makes external calls, creates a new . `Envelope``Envelope`

**Backend**
5. **Middleware (Response)**: Transforms the return on its way back. `Envelope`
6. :
   - Converts the response into an HTTP response. `Envelope`

**Endpoint**
7. : Sends the response back to the client. **Router**
