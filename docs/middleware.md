# Middleware

Middleware allows for authenticating, modifying or transforming the Envelope.

## Middleware types

### Authentication

#### Basic Auth
Validates a `username` and `password` combination, often passed as Base64-encoded credentials in an HTTP header. `Authorization`

- **`username`**: A string representing the username to authenticate the request.
- **`password`**: A string representing the password corresponding to the username.
- **`token_path`** (optional): A path or endpoint where tokens or credentials are stored or validated.

#### JWT Auth
To authenticate a request using JWT (JSON Web Token), the following properties are typically required:

- **`jwt_secret`** (or a public key): A secret key used to verify the JWT signature. This might alternatively reference a public/private key pair depending on the algorithm.
- **`issuer`** (optional): The expected "issuer" of the token (found in the token's claims).
- **`audience`** (optional): The expected target audience of the token (found in the token's claims).
- **`token_path`** (optional): A path or endpoint to validate or obtain JWTs.
- **`header`**: The header where the token is expected to be present (e.g., with a `Bearer <token>` format). `Authorization`

### Transformation

#### DICOM to JSON
Converts DICOM into JSON

#### DICOMweb to JSON
Converts DICOMweb into JSON

#### FHIR to DICOM
Converts FHIR to DICOM

#### JMIX to DICOM
Converts JMIX to DICOM
