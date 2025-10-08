# JMIX Envelope Security White Paper
Christopher Skene
8th June 2025
DRAFT - WORK IN PROGRESS

## Abstract
This paper presents the security encryption architecture for JMIX (JSON Medical Imaging Exchange) Envelopes, supporting secure, peer-to-peer exchange of identifiable imaging data. The model is designed to support interoperability between independent actors and optionally leverage the Aurabox directory and services for enhanced usability, policy enforcement, and auditability, without introducing any dependency on Aurabox for decryption or transmission.

## 1. Introduction
Healthcare imaging data exchange faces challenges around security, interoperability, and decentralisation. Traditional centralised exchange models introduce unnecessary dependencies and trust anchors, whereas pure peer-to-peer systems can lack usability or governance.

JMIX aims to strike a balance: enabling direct, secure, peer-to-peer sharing, while allowing parties who choose to use a trusted directory (e.g. Aurabox) to gain benefits like key discovery, consent validation, and identity anchoring.

Some of the features of this security model include:
- Forward Secrecy: Ephemeral keys ensure compromise of long-term keys does not expose past data.
- Authentication: GCM tag ensures ciphertext cannot be tampered with.
- Minimal Trust: No dependency on Aurabox or any other service for basic operation.
- Optional Governance: Aurabox can be used to anchor identities, resolve key IDs, or audit transfers
- Non-repudiation: Proof of sender

## 2. Design Principles
Self-contained encryption: All JMIX envelopes must be decryptable using only what is contained inside the envelope, assuming the recipient possesses the correct private key.

Ephemeral key use: All encryption must use ephemeral sender keys to provide forward secrecy.

Fixed cryptographic stack: All payloads are encrypted using AES-256-GCM with ECDH key agreement and HKDF (SHA-256) for symmetric key derivation.

Directory-optional context: Key discovery, identity mapping, and policy evaluation may optionally be facilitated by Aurabox or a compatible directory, but are not required for decryption.

## 3. Cryptographic Foundations
Algorithm:
- Encryption: AES-256 in Galois/Counter Mode (GCM)
- Key Exchange: Elliptic Curve Diffie-Hellman (ECDH) over Curve25519
- Key Derivation: HKDF using SHA-256

Each package is encrypted using a one-time symmetric key derived from ECDH, ensuring forward secrecy and resilience.

## 4. Envelope Structure

### a. Encryption (required when the package is encrypted)
The Encryption block describes the encryption of the package directory. It is required when the package is encrypted, but not when the package is unencrypted (for example, if the envelope is being used as a download).

```json
"encryption": {
  "algorithm": "AES-256-GCM",
  "ephemeral_public_key": "<base64>",
  "iv": "<base64>",
  "auth_tag": "<base64>"
}
```

These fields are all required for the recipient to decrypt the contents of the package directory.
- algorithm: Always AES-256-GCM
- ephemeral_public_key: The sender's one-time-use ECDH public key.
- iv: 96-bit AES-GCM nonce.
- auth_tag: 128-bit authentication tag generated during encryption.

### b. Sender Assertion (optional, but recommended)
The Sender Assertion block allows the sender to provide information about themselves, so that the receiver can verify the authenticity of the package. It serves as a mechanism for authenticating and verifying the identity of the sender of a JMIX envelope, especially in decentralised or federated environments.

It helps guard against spoofing by cryptographically binding the claimed identity to the encrypted payload.
Enables trust decisions in the absence of live infrastructure (e.g., offline environments).
Supports forensic verification: recipients can prove to others who sent the data and that it hasn’t been altered.

The sender_assertion block exists to:
- Provide verifiable metadata about the sender, such as their claimed identity and the cryptographic evidence that supports that claim.
- Enable trust and non-repudiation, by binding the sender’s claimed identity to the specific encrypted payload using a signature.
- Support optional directory-based verification, such as through Aurabox or other compatible directories, without being dependent on them.

This is crucial in peer-to-peer settings, where there is no central server to authenticate participants. It allows recipients and third parties (for example, auditors) to validate who sent a JMIX package, even offline. The Sender Assertion is foundational to JMIX's trust and governance model, acting as a decentralised and optionally third-party-verified sender identity proof, ensuring secure and accountable medical imaging exchange.

```json
{
  "signing_key": {
    "alg": "Ed25519",
    "public_key": "<base64>",
    "fingerprint": "SHA256:<hex>"
  },
  "key_reference": "aurabox://org/clinic-a#key-ed25519",
  "signed_fields": [
    "sender.id",
    "sender.name",
    "signatures.manifest.kid",
    "manifest_hash"
  ],
  "signature": "<base64sig>",
  "expires_at": "2025-07-07T00:00:00Z",
  "directory_attestation": {
    "provider": "aurabox",
    "attestation_signature": "<JWS>",
    "attestation_timestamp": "2025-06-07T14:01:00Z",
    "attestation_public_key": "<base64>"
  }
}
```

The file is referenced in the sender block, as follows:

```json
"sender": {
  "id": "org:au.gov.health.123456",
  "name": "Clinic A",
  "contact": "dicom@clinica.org.au",
  "assertion": {
    "...": "assertion goes here"
  }
}
```

While the attestation includes expiry, the receiver is not bound to reject the envelope if the expiry has passed. This is up to receivers to define.

### c. Requester Assertion (optional)
The requester assertion follows a similar pattern to the Sender Assertion and allows the receiver to validate the requester. This may be useful to determine that the receiver is receiving information it is allowed to hold, based on the requester's identity.

The requester identity should be provided by the requester when requesting the data, either via a JMIX API or some other mechanism. The signature model is identical to the sender's.

```json
"requester": {
  "name": "Dr. Michael Chen",
  "email": "mchen@university-medical.edu",
  "authentication_level": "directory_verified",
  "assertion": {
    "...": "assertion goes here"
  }
}
```

### Verification Workflow
- Fetch signed_fields and corresponding values from the envelope.
- Canonicalise the field-value structure.
- Verify the signature using signing_key.public_key.
- Optional: Validate directory_attestation.attestation_signature using attestation_public_key.
- Check expires_at if present.

## 5. Modes of Operation

| Use Case | Description |
| --- | --- |
| Pure P2P | Sender and recipient exchange directly (email, file transfer, etc.). Only envelope metadata is required. |
| P2P with Aurabox resolution | Recipient or sender uses Aurabox API to discover keys, validate sender IDs, or log consent. |
| Asymmetric P2P | One user uses Aurabox for discovery, the other operates independently. |

## 6. Encryption Workflow
- Sender generates a new ephemeral EC keypair.
- Shared key is derived via ECDH (sender's ephemeral privkey plus recipient's pubkey).
- Symmetric key is produced using HKDF with SHA-256.
- Payload is encrypted with AES-256-GCM using the derived key, random IV, and the tag is retained.
- Envelope is assembled with the encrypted payload and manifest.json.

## 7. Recipient Key Management and Discovery
JMIX assumes that each recipient maintains a stable, long-term asymmetric keypair (for example, Curve25519) for envelope decryption. The public component of this key must be made available to potential senders through one or more of the following mechanisms:
- Out-of-Band Exchange: Recipients may distribute their public keys manually via QR codes, shared config files, or email. This is suitable for small, trusted collaborations.
- Directory Lookup (optional): Aurabox and compatible directories may expose a public key endpoint, for example, aurabox://org/clinic-a#key-ed25519, to facilitate key discovery and resolution.
- In-Envelope Reference: Senders may embed a key_reference URI in the sender assertion, indicating the expected recipient key. This does not impact decryption, but helps with auditing and debugging.

For secure operation, recipients must safeguard their private key material. Rotation or revocation of keys should be managed via external policy or infrastructure and is not currently defined within the JMIX envelope format.

## 8. Deployment Scenarios
- Independent Users: Healthcare providers, researchers, or clinicians using custom tooling.
- Integrated Aurabox Users: Those who opt-in for identity resolution, central consent records, or integration with Aurabox storage and viewing services.
- Mixed Mode: One peer uses Aurabox to obtain the other’s public key, then sends a self-contained envelope.

## 9. Conclusion
The JMIX envelope encryption strategy strikes a balance between secure, verifiable data exchange and ecosystem flexibility. It provides robust default behaviour for independent users while allowing powerful enhancements when Aurabox services are available.

This ensures imaging data can be safely exchanged, whether between two doctors, hospitals, or platforms, with confidence in its privacy, integrity, and interoperability.
