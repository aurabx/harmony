# Router description

The correct behaviour of the router should be:

## Incoming requests

1. The request is received by the router on a given path.
2. The router gets the endpoint for the path, and the group for the endpoint.
3. The endpoint transforms the request if necessary (usually to JSON)
4. If the group defines incoming middleware, the router applies it.
5. The router sends the request to the backend, which sends it to the internal service.
then...
6. The router receives the response from the service and selects the appropriate backend.
7. The backend transforms the response if necessary (usually to JSON)
8. If the group defines outgoing middleware, the router applies it.
9. The router sends the response to the endpoint.
10. The endpoint sends the response to the client.

## Outgoing requests

1. The request is received by the router on a given path or connection
2. The router gets the backend and group for the path
3. The backend transforms the request if necessary (usually to JSON)
4. If the group defines outgoing middleware, the router applies it.
5. The router sends the request to the groups peer, which sends it to the external service.
   then...
6. The router receives the response from the service and selects the appropriate peer.
7. The peer transforms the response if necessary (usually to JSON)
8. If the group defines incoming middleware, the router applies it.
9. The router sends the response to the backend.
10. The backend transforms the response if necessary (usually to JSON) and sends it to the service.