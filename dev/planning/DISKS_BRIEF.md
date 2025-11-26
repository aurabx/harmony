# Concept for a Disk Backend in Harmony

## Overview
A disk backend should behave like any other Harmony backend, but instead of sending or receiving data over HTTP it reads and writes to the filesystem. To achieve this cleanly, treat disk endpoints as addressable backends with a pluggable resolver that maps a logical request to a concrete filesystem path.

## Backend Address Abstraction
Introduce a generic logical address for any backend.

• http: `http://service-x.internal/api/foo`  
• disk: `disk://local-archive/patient-imaging`  
• s3: `s3://bucket/path`

Harmony routes based on an opaque address object rather than raw URLs or paths. A disk address such as `disk://local-archive/patient-imaging` is passed to the disk backend, which decides how to interpret it.

## Disk Backend Driver and Path Resolver
Add a dedicated disk backend with a resolver interface. The resolver takes:

• BackendAddress  
• Context (tenant, connection, JMIX metadata, direction)

It returns:

• Concrete filesystem path  
• Required operation (read or write)

Examples:

• resolveWritePath  
• resolveReadPath

The resolver can be configured per backend instance so different disk backends can use different path strategies.

## Template Based Path Mapping
A template driven approach covers most use cases. Backend config might include:

• root: `/mnt/harmony/local-archive`  
• write_pattern: `{tenant}/{connection}/{timestamp}_{correlationId}.{ext}`  
• read_pattern: `{storedPath}`

Useful fields include:

• tenant, org, connection  
• studyInstanceUid, seriesInstanceUid, sopInstanceUid  
• jmix.id, jmix.type  
• timestamp, uuid, hash

The resolver renders the pattern using the context and joins it to the root path.

## First Class Disk Locations in Responses
When writing to disk, callers need a way to read the same item later. Two strategies:

• Return a location URI such as `disk://local-archive/patient-imaging?path=tenantA/.../file.dcm`  
• Maintain a Harmony-managed ID mapped to a stored path

URI based locations keep the system stateless. IDs allow for lifecycle management and auditing.

## Routing Rules
Disk endpoints should work exactly like HTTP endpoints. Routing rules match JMIX envelope fields and choose a backend. The scheme determines the driver:

• http:// → HttpBackend  
• disk:// → DiskBackend  
• dicom:// could be added later

## Staging vs Archive Modes
Disk use cases differ, so consider two modes.

• Staging: temporary files written then relayed to another backend, with cleanup via TTL  
• Archive: long term storage with stable identifiers or locations

The backend config should specify which mode is intended.

## Multi Tenant and Connection Based Isolation
To ensure tenant isolation, incorporate identifiers in the path mapping:

• tenant or organisation  
• connection id  
• endpoint keys from provisioning  
• JMIX envelope metadata

Example pattern:

• `{tenant}/{receiverKey}/{studyInstanceUid}/{seriesInstanceUid}/{sopInstanceUid}.dcm`

## Pluggable Key to Path Resolver
Define a small interface that computes a relative path key.

• resolveWriteKey(context) → key  
• resolveReadKey(context) → key

Then map it to `root/key`.

Possible resolvers:

• TemplateResolver  
• HashShardingResolver  
• DatabaseLookupResolver

This allows Harmony to support multiple mapping strategies without changing the core.

## Safety Considerations
Ensure robust behaviour by enforcing:

• Path normalisation and validation to prevent traversal  
• Atomic writes using temporary files then rename  
• Clear separation of readable and writable roots  
• Accurate error handling for missing files, permissions or disk space
