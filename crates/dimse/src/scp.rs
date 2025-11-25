//! Service Class Provider (SCP) implementation for inbound DIMSE operations

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_encoding::text::SpecificCharacterSet;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::{InMemDicomObject, StandardDataDictionary};
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::association::server::{AcceptAny, ServerAssociationOptions};
use dicom_ul::{Pdu, ServerAssociation};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{debug, error, info, span, warn, Level};

use crate::config::DimseConfig;
use crate::router::{DimseRequest, DimseRequestPayload, DimseResponse, Router};
use crate::types::{DatasetStream, QueryLevel};
use crate::{DimseError, Result};

/// Sub-operation counts for C-MOVE and C-GET responses
struct SubOperationCounts {
    remaining: u16,
    completed: u16,
    failed: u16,
    warning: u16,
}

/// Trait for providing query capabilities to the SCP
#[async_trait]
pub trait QueryProvider: Send + Sync {
    /// Find datasets matching the given query
    async fn find(
        &self,
        query_level: QueryLevel,
        parameters: &std::collections::HashMap<String, String>,
        max_results: u32,
    ) -> Result<Vec<DatasetStream>>;

    /// Locate datasets for move operations
    async fn locate(
        &self,
        query_level: QueryLevel,
        parameters: &std::collections::HashMap<String, String>,
    ) -> Result<Vec<DatasetStream>>;

    /// Retrieve datasets for get operations (C-GET)
    async fn get(
        &self,
        query_level: QueryLevel,
        parameters: &std::collections::HashMap<String, String>,
    ) -> Result<Vec<DatasetStream>>;

    /// Store a dataset (for C-STORE operations)
    async fn store(&self, dataset: DatasetStream) -> Result<()>;
}

/// DIMSE Service Class Provider
pub struct DimseScp {
    config: DimseConfig,
    #[allow(dead_code)]
    query_provider: Arc<dyn QueryProvider>, // TODO: Used for database queries
    router: Option<Arc<dyn Router>>,
    active_associations: Arc<RwLock<u32>>,
}

impl DimseScp {
    /// Create a new SCP with the given configuration and query provider
    pub fn new(config: DimseConfig, query_provider: Arc<dyn QueryProvider>) -> Self {
        Self {
            config,
            query_provider,
            router: None,
            active_associations: Arc::new(RwLock::new(0)),
        }
    }

    /// Set the router for handling requests
    pub fn with_router(mut self, router: Arc<dyn Router>) -> Self {
        self.router = Some(router);
        self
    }

    /// Start the SCP listener
    pub async fn run(self, shutdown: tokio_util::sync::CancellationToken) -> Result<()> {
        let addr = SocketAddr::new(self.config.bind_addr, self.config.port);
        // Use socket2 for reliable bind with SO_REUSEADDR (helps with ephemeral port races in tests)
        let socket = socket2::Socket::new(
            match addr {
                SocketAddr::V4(_) => socket2::Domain::IPV4,
                SocketAddr::V6(_) => socket2::Domain::IPV6,
            },
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )?;
        socket.set_reuse_address(true)?;
        socket.bind(&addr.into())?;
        socket.listen(128)?;
        let std_listener: std::net::TcpListener = socket.into();
        std_listener.set_nonblocking(true)?;
        let listener = TcpListener::from_std(std_listener)?;

        info!(
            "Starting DIMSE SCP on {} (AET: {})",
            addr, self.config.local_aet
        );

        // Validate configuration
        self.config.validate()?;

        let scp = Arc::new(self);

        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, peer_addr)) => {
                            debug!("Accepted connection from {}", peer_addr);

                            // Check association limit
                            {
                                let active = scp.active_associations.read().await;
                                if *active >= scp.config.max_associations {
                                    warn!(
                                        "Maximum associations reached, rejecting connection from {}",
                                        peer_addr
                                    );
                                    drop(stream);
                                    continue;
                                }
                            }

                            let scp_clone = Arc::clone(&scp);
                            tokio::spawn(async move {
                                if let Err(e) = scp_clone.handle_association(stream, peer_addr).await {
                                    // "Connection closed by peer" during handshake is usually a health check
                                    let err_msg = e.to_string();
                                    if err_msg.contains("Connection closed by peer") {
                                        debug!("Connection from {} closed during handshake (likely health check)", peer_addr);
                                    } else {
                                        error!("Error handling association from {}: {}", peer_addr, e);
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            error!("Error accepting connection: {}", e);
                        }
                    }
                }
                _ = shutdown.cancelled() => {
                    info!("Shutdown signal received, stopping SCP listener");
                    break;
                }
            }
        }

        info!("DIMSE SCP listener stopped");
        Ok(())
    }

    /// Handle a single association
    async fn handle_association(
        &self,
        stream: tokio::net::TcpStream,
        peer_addr: SocketAddr,
    ) -> Result<()> {
        // Increment active associations
        {
            let mut active = self.active_associations.write().await;
            *active += 1;
        }

        let result = self.handle_association_inner(stream, peer_addr).await;

        // Decrement active associations
        {
            let mut active = self.active_associations.write().await;
            *active -= 1;
        }

        result
    }

    /// Inner association handler
    async fn handle_association_inner(
        &self,
        stream: tokio::net::TcpStream,
        peer_addr: SocketAddr,
    ) -> Result<()> {
        info!("Starting association with {}", peer_addr);

        // Build server association options based on config
        // Use promiscuous mode to accept any presentation context
        let scp_options = ServerAssociationOptions::new()
            .ae_title(self.config.local_aet.as_str())
            .ae_access_control(AcceptAny)
            .promiscuous(true);

        // Establish the association
        let mut association = scp_options.establish_async(stream).await.map_err(|e| {
            DimseError::association(format!("Failed to establish association: {}", e))
        })?;

        info!(
            "Association established with {} (calling AET: {})",
            peer_addr,
            association.client_ae_title()
        );

        // Buffer for accumulating identifier data across multiple PDUs
        let mut pending_command: Option<(u16, u16)> = None; // (command_field, message_id)
        let mut accumulated_identifier = Vec::new();

        // Process PDUs until association is released or aborted
        loop {
            match association.receive().await {
                Ok(Pdu::PData { data }) => {
                    // Handle P-DATA PDU containing DIMSE commands
                    if let Err(e) = self
                        .handle_pdata(
                            &mut association,
                            data,
                            &mut pending_command,
                            &mut accumulated_identifier,
                        )
                        .await
                    {
                        error!("Error handling P-DATA: {}", e);
                        // Send abort and break
                        let _ = association.abort().await;
                        break;
                    }
                }
                Ok(Pdu::ReleaseRQ) => {
                    // Association release requested by SCU
                    info!("Association release requested by {}", peer_addr);
                    association.send(&Pdu::ReleaseRP).await.map_err(|e| {
                        DimseError::network(format!("Failed to send release: {}", e))
                    })?;
                    break;
                }
                Ok(Pdu::AbortRQ { source }) => {
                    // Association aborted by SCU
                    warn!("Association aborted by {}: {:?}", peer_addr, source);
                    break;
                }
                Ok(pdu) => {
                    // Unexpected PDU
                    warn!("Unexpected PDU received: {:?}", pdu);
                    let _ = association.abort().await;
                    break;
                }
                Err(e) => {
                    error!("Error receiving PDU from {}: {}", peer_addr, e);
                    let _ = association.abort().await;
                    break;
                }
            }
        }

        info!("Association with {} completed", peer_addr);
        Ok(())
    }

    /// Handle P-DATA PDU containing DIMSE command
    async fn handle_pdata(
        &self,
        association: &mut ServerAssociation<tokio::net::TcpStream>,
        pdata: Vec<dicom_ul::pdu::PDataValue>,
        pending_command: &mut Option<(u16, u16)>,
        accumulated_identifier: &mut Vec<u8>,
    ) -> Result<()> {
        // Separate command and data PDUs, track presentation context ID
        let mut command_data = Vec::new();
        let mut identifier_data = Vec::new();
        let mut presentation_context_id = 1u8; // Default to 1

        for pdata_value in pdata {
            presentation_context_id = pdata_value.presentation_context_id;
            match pdata_value.value_type {
                dicom_ul::pdu::PDataValueType::Command => {
                    command_data.extend_from_slice(&pdata_value.data);
                }
                dicom_ul::pdu::PDataValueType::Data => {
                    identifier_data.extend_from_slice(&pdata_value.data);
                }
            }
        }

        // Check if we have command data
        if command_data.is_empty() {
            // This is a data-only PDU - accumulate it for pending command
            if !identifier_data.is_empty() {
                debug!(
                    "Received data-only P-DATA ({} bytes), accumulating",
                    identifier_data.len()
                );
                accumulated_identifier.extend_from_slice(&identifier_data);

                // Check if we have a pending command to dispatch
                if let Some((command_field, message_id)) = *pending_command {
                    // Dispatch the command with accumulated data
                    debug!(
                        "Dispatching pending command 0x{:04X} with {} bytes of data",
                        command_field,
                        accumulated_identifier.len()
                    );
                    self.dispatch_command(
                        association,
                        command_field,
                        message_id,
                        accumulated_identifier.clone(),
                        presentation_context_id,
                    )
                    .await?;
                    *pending_command = None;
                    accumulated_identifier.clear();
                }
            }
            return Ok(());
        }

        // Parse the command dataset using Implicit VR Little Endian (DICOM command PDUs use this)
        let ts = TransferSyntaxRegistry
            .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
            .ok_or_else(|| DimseError::parse("Implicit VR Little Endian TS not found"))?;

        let cursor = std::io::Cursor::new(&command_data);
        let command_obj = InMemDicomObject::<StandardDataDictionary>::read_dataset_with_ts_cs(
            cursor,
            ts,
            SpecificCharacterSet::default(),
        )
        .map_err(|e| {
            DimseError::parse(format!(
                "Failed to parse command dataset ({} bytes): {}",
                command_data.len(),
                e
            ))
        })?;

        // Extract command field to determine operation type
        let command_field = command_obj
            .element(tags::COMMAND_FIELD)
            .map_err(|_| DimseError::parse("Missing command field"))?
            .uint16()
            .map_err(|e| DimseError::parse(format!("Invalid command field: {}", e)))?;

        // Extract message ID for response correlation
        let message_id = command_obj
            .element(tags::MESSAGE_ID)
            .map_err(|_| DimseError::parse("Missing message ID"))?
            .uint16()
            .map_err(|e| DimseError::parse(format!("Invalid message ID: {}", e)))?;

        debug!(
            "Received DIMSE command: 0x{:04X}, message ID: {}",
            command_field, message_id
        );

        // Check if this command expects a dataset
        let expects_dataset = command_obj
            .element(tags::COMMAND_DATA_SET_TYPE)
            .ok()
            .and_then(|e| e.uint16().ok())
            .map(|v| v != 0x0101) // 0x0101 = no dataset present
            .unwrap_or(false);

        // If we have identifier data in this PDU, use it immediately
        if !identifier_data.is_empty() {
            debug!(
                "Command has {} bytes of identifier data in same PDU",
                identifier_data.len()
            );
            return self
                .dispatch_command(
                    association,
                    command_field,
                    message_id,
                    identifier_data,
                    presentation_context_id,
                )
                .await;
        }

        // If command expects dataset but we don't have it yet, buffer the command
        if expects_dataset {
            debug!("Command expects dataset, buffering command for next PDU");
            *pending_command = Some((command_field, message_id));
            return Ok(());
        }

        // No dataset expected, dispatch immediately
        self.dispatch_command(
            association,
            command_field,
            message_id,
            Vec::new(),
            presentation_context_id,
        )
        .await
    }

    /// Dispatch a DIMSE command to the appropriate handler
    async fn dispatch_command(
        &self,
        association: &mut ServerAssociation<tokio::net::TcpStream>,
        command_field: u16,
        message_id: u16,
        identifier_data: Vec<u8>,
        presentation_context_id: u8,
    ) -> Result<()> {
        // Dispatch based on command type
        match command_field {
            0x0030 => {
                // C-ECHO-RQ
                self.handle_c_echo(association, message_id, presentation_context_id)
                    .await
            }
            0x0020 => {
                // C-FIND-RQ
                self.handle_c_find(
                    association,
                    message_id,
                    identifier_data,
                    presentation_context_id,
                )
                .await
            }
            0x0021 => {
                // C-MOVE-RQ
                self.handle_c_move(
                    association,
                    message_id,
                    identifier_data,
                    presentation_context_id,
                )
                .await
            }
            0x0010 => {
                // C-GET-RQ
                self.handle_c_get(
                    association,
                    message_id,
                    identifier_data,
                    presentation_context_id,
                )
                .await
            }
            0x0001 => {
                // C-STORE-RQ
                self.handle_c_store(
                    association,
                    message_id,
                    identifier_data,
                    presentation_context_id,
                )
                .await
            }
            _ => {
                warn!("Unknown DIMSE command: 0x{:04X}", command_field);
                Err(DimseError::operation_failed(format!(
                    "Unsupported command: 0x{:04X}",
                    command_field
                )))
            }
        }
    }

    /// Handle C-ECHO request
    async fn handle_c_echo(
        &self,
        association: &mut ServerAssociation<tokio::net::TcpStream>,
        message_id: u16,
        presentation_context_id: u8,
    ) -> Result<()> {
        if !self.config.enable_echo {
            return Err(DimseError::operation_failed("C-ECHO not enabled"));
        }

        debug!("Handling C-ECHO request (message ID: {})", message_id);

        // Build C-ECHO-RSP command dataset
        let mut response = InMemDicomObject::new_empty();

        // Command Field (0000,0100) = 0x8030 (C-ECHO-RSP)
        response.put(DataElement::new(
            tags::COMMAND_FIELD,
            VR::US,
            PrimitiveValue::from(0x8030u16),
        ));

        // Message ID Being Responded To (0000,0120)
        response.put(DataElement::new(
            tags::MESSAGE_ID_BEING_RESPONDED_TO,
            VR::US,
            PrimitiveValue::from(message_id),
        ));

        // Command Data Set Type (0000,0800) = 0x0101 (no dataset)
        response.put(DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            PrimitiveValue::from(0x0101u16),
        ));

        // Status (0000,0900) = 0x0000 (Success)
        response.put(DataElement::new(
            tags::STATUS,
            VR::US,
            PrimitiveValue::from(0x0000u16),
        ));

        // Affected SOP Class UID (0000,0002) = Verification SOP Class
        response.put(DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            PrimitiveValue::from("1.2.840.10008.1.1"),
        ));

        // Encode response to bytes using Implicit VR Little Endian
        let mut response_bytes = Vec::new();
        let ts = TransferSyntaxRegistry
            .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
            .ok_or_else(|| {
                DimseError::operation_failed("Implicit VR Little Endian TS not found")
            })?;

        response
            .write_dataset_with_ts(&mut response_bytes, ts)
            .map_err(|e| {
                DimseError::operation_failed(format!("Failed to encode response: {}", e))
            })?;

        // Send as P-DATA PDU
        let pdata_value = dicom_ul::pdu::PDataValue {
            presentation_context_id,
            value_type: dicom_ul::pdu::PDataValueType::Command,
            is_last: true,
            data: response_bytes,
        };

        association
            .send(&Pdu::PData {
                data: vec![pdata_value],
            })
            .await
            .map_err(|e| DimseError::network(format!("Failed to send C-ECHO response: {}", e)))?;

        info!("C-ECHO response sent successfully");
        Ok(())
    }

    /// Handle C-FIND request
    async fn handle_c_find(
        &self,
        association: &mut ServerAssociation<tokio::net::TcpStream>,
        message_id: u16,
        identifier_data: Vec<u8>,
        presentation_context_id: u8,
    ) -> Result<()> {
        if !self.config.enable_find {
            return Err(DimseError::operation_failed("C-FIND not enabled"));
        }

        debug!(
            "Handling C-FIND request (message ID: {}, identifier size: {} bytes)",
            message_id,
            identifier_data.len()
        );

        // Check if we have identifier data
        if identifier_data.is_empty() {
            warn!("C-FIND request has no identifier data");
            // Send failure response
            self.send_cfind_response(
                association,
                message_id,
                0xC000, // Failure
                None,
            )
            .await?;
            return Ok(());
        }

        // Get the transfer syntax for this presentation context
        let ts = association
            .presentation_contexts()
            .iter()
            .find(|pc| pc.id == presentation_context_id)
            .and_then(|pc| TransferSyntaxRegistry.get(&pc.transfer_syntax))
            .ok_or_else(|| {
                DimseError::parse(format!(
                    "Transfer syntax not found for presentation context {}",
                    presentation_context_id
                ))
            })?;

        let cursor = std::io::Cursor::new(&identifier_data);
        let identifier = InMemDicomObject::<StandardDataDictionary>::read_dataset_with_ts_cs(
            cursor,
            ts,
            SpecificCharacterSet::default(),
        )
        .map_err(|e| DimseError::parse(format!("Failed to parse identifier dataset: {}", e)))?;

        // Extract query level from QueryRetrieveLevel (0008,0052)
        let query_level_str = identifier
            .element_by_name("QueryRetrieveLevel")
            .ok()
            .and_then(|e| e.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "STUDY".to_string());

        let query_level = query_level_str
            .parse::<QueryLevel>()
            .unwrap_or(QueryLevel::Study);

        // Extract query parameters from the identifier
        let mut parameters = std::collections::HashMap::new();

        // Common DICOM query tags
        let query_tags = vec![
            ("PatientID", "00100020"),
            ("PatientName", "00100010"),
            ("StudyInstanceUID", "0020000D"),
            ("SeriesInstanceUID", "0020000E"),
            ("SOPInstanceUID", "00080018"),
            ("StudyDate", "00080020"),
            ("StudyTime", "00080030"),
            ("Modality", "00080060"),
            ("AccessionNumber", "00080050"),
        ];

        for (name, _tag) in query_tags {
            if let Ok(elem) = identifier.element_by_name(name) {
                if let Ok(value) = elem.to_str() {
                    if !value.is_empty() {
                        parameters.insert(name.to_string(), value.to_string());
                    }
                }
            }
        }

        debug!(
            "C-FIND query: level={}, params={:?}",
            query_level, parameters
        );

        // Query the provider
        match self.query_provider.find(query_level, &parameters, 0).await {
            Ok(datasets) => {
                debug!("Found {} matching datasets", datasets.len());

                // Send each dataset as a pending response (status 0xFF00)
                for dataset in &datasets {
                    self.send_cfind_response(
                        association,
                        message_id,
                        0xFF00, // Pending
                        Some(dataset),
                    )
                    .await?;
                }

                // Send final success response (status 0x0000)
                self.send_cfind_response(
                    association,
                    message_id,
                    0x0000, // Success
                    None,
                )
                .await?;

                info!("C-FIND completed with {} results", datasets.len());
                Ok(())
            }
            Err(e) => {
                error!("C-FIND query failed: {}", e);
                // Send failure response (status 0xC000)
                self.send_cfind_response(
                    association,
                    message_id,
                    0xC000, // Failure
                    None,
                )
                .await?;
                Ok(())
            }
        }
    }

    /// Send a C-FIND response
    async fn send_cfind_response(
        &self,
        association: &mut ServerAssociation<tokio::net::TcpStream>,
        message_id: u16,
        status: u16,
        dataset: Option<&crate::types::DatasetStream>,
    ) -> Result<()> {
        // Build C-FIND-RSP command dataset
        let mut response = InMemDicomObject::new_empty();

        // Command Field (0000,0100) = 0x8020 (C-FIND-RSP)
        response.put(DataElement::new(
            tags::COMMAND_FIELD,
            VR::US,
            PrimitiveValue::from(0x8020u16),
        ));

        // Message ID Being Responded To (0000,0120)
        response.put(DataElement::new(
            tags::MESSAGE_ID_BEING_RESPONDED_TO,
            VR::US,
            PrimitiveValue::from(message_id),
        ));

        // Command Data Set Type (0000,0800)
        let has_dataset = dataset.is_some();
        response.put(DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            PrimitiveValue::from(if has_dataset { 0x0000u16 } else { 0x0101u16 }),
        ));

        // Status (0000,0900)
        response.put(DataElement::new(
            tags::STATUS,
            VR::US,
            PrimitiveValue::from(status),
        ));

        // Affected SOP Class UID (0000,0002) = Study Root Query/Retrieve - FIND
        response.put(DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            PrimitiveValue::from("1.2.840.10008.5.1.4.1.2.2.1"),
        ));

        // Encode command response
        let ts = TransferSyntaxRegistry
            .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
            .ok_or_else(|| {
                DimseError::operation_failed("Implicit VR Little Endian TS not found")
            })?;

        let mut response_bytes = Vec::new();
        response
            .write_dataset_with_ts(&mut response_bytes, ts)
            .map_err(|e| {
                DimseError::operation_failed(format!("Failed to encode response: {}", e))
            })?;

        let command_pdata = dicom_ul::pdu::PDataValue {
            presentation_context_id: 1,
            value_type: dicom_ul::pdu::PDataValueType::Command,
            is_last: !has_dataset,
            data: response_bytes,
        };

        // If we have a dataset, send it as well
        if let Some(ds) = dataset {
            // Convert the dataset to a DICOM object
            let dicom_obj = ds.to_object().await?;

            // Encode the identifier dataset
            let mut identifier_bytes = Vec::new();
            dicom_obj
                .write_dataset_with_ts(&mut identifier_bytes, ts)
                .map_err(|e| {
                    DimseError::operation_failed(format!("Failed to encode identifier: {}", e))
                })?;

            let data_pdata = dicom_ul::pdu::PDataValue {
                presentation_context_id: 1,
                value_type: dicom_ul::pdu::PDataValueType::Data,
                is_last: true,
                data: identifier_bytes,
            };

            association
                .send(&Pdu::PData {
                    data: vec![command_pdata, data_pdata],
                })
                .await
                .map_err(|e| {
                    DimseError::network(format!("Failed to send C-FIND response: {}", e))
                })?;
        } else {
            association
                .send(&Pdu::PData {
                    data: vec![command_pdata],
                })
                .await
                .map_err(|e| {
                    DimseError::network(format!("Failed to send C-FIND response: {}", e))
                })?;
        }

        Ok(())
    }

    /// Handle C-MOVE request (stub - not fully implemented)
    async fn handle_c_move(
        &self,
        association: &mut ServerAssociation<tokio::net::TcpStream>,
        message_id: u16,
        identifier_data: Vec<u8>,
        _presentation_context_id: u8,
    ) -> Result<()> {
        if !self.config.enable_move {
            return Err(DimseError::operation_failed("C-MOVE not enabled"));
        }

        debug!(
            "Handling C-MOVE request (message ID: {}, identifier size: {} bytes)",
            message_id,
            identifier_data.len()
        );
        warn!("C-MOVE operation not fully implemented - returning 'Unable to perform sub-operations' status");

        // Parse identifier to log query info
        if !identifier_data.is_empty() {
            let ts = TransferSyntaxRegistry
                .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
                .ok_or_else(|| DimseError::parse("Implicit VR Little Endian TS not found"))?;

            if let Ok(identifier) =
                InMemDicomObject::<StandardDataDictionary>::read_dataset_with_ts_cs(
                    std::io::Cursor::new(&identifier_data),
                    ts,
                    SpecificCharacterSet::default(),
                )
            {
                // Try to extract move destination
                if let Ok(dest) = identifier.element_by_name("MoveDestination") {
                    if let Ok(dest_aet) = dest.to_str() {
                        debug!("C-MOVE destination AET: {}", dest_aet);
                    }
                }
            }
        }

        // Send failure response with "Unable to perform sub-operations" status
        // Status 0xA702 = Unable to perform sub-operations
        self.send_cmove_response(
            association,
            message_id,
            0xA702,
            SubOperationCounts {
                remaining: 0,
                completed: 0,
                failed: 0,
                warning: 0,
            },
        )
        .await?;

        info!("C-MOVE request handled with 'not implemented' status");
        Ok(())
    }

    /// Send a C-MOVE response
    async fn send_cmove_response(
        &self,
        association: &mut ServerAssociation<tokio::net::TcpStream>,
        message_id: u16,
        status: u16,
        counts: SubOperationCounts,
    ) -> Result<()> {
        let mut response = InMemDicomObject::new_empty();

        // Command Field (0000,0100) = 0x8021 (C-MOVE-RSP)
        response.put(DataElement::new(
            tags::COMMAND_FIELD,
            VR::US,
            PrimitiveValue::from(0x8021u16),
        ));

        // Message ID Being Responded To (0000,0120)
        response.put(DataElement::new(
            tags::MESSAGE_ID_BEING_RESPONDED_TO,
            VR::US,
            PrimitiveValue::from(message_id),
        ));

        // Command Data Set Type (0000,0800) = 0x0101 (no dataset)
        response.put(DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            PrimitiveValue::from(0x0101u16),
        ));

        // Status (0000,0900)
        response.put(DataElement::new(
            tags::STATUS,
            VR::US,
            PrimitiveValue::from(status),
        ));

        // Affected SOP Class UID (0000,0002) = Study Root Query/Retrieve - MOVE
        response.put(DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            PrimitiveValue::from("1.2.840.10008.5.1.4.1.2.2.2"),
        ));

        // Add sub-operation status fields
        // Number of Remaining Sub-operations (0000,1020)
        response.put(DataElement::new(
            tags::NUMBER_OF_REMAINING_SUBOPERATIONS,
            VR::US,
            PrimitiveValue::from(counts.remaining),
        ));

        // Number of Completed Sub-operations (0000,1021)
        response.put(DataElement::new(
            tags::NUMBER_OF_COMPLETED_SUBOPERATIONS,
            VR::US,
            PrimitiveValue::from(counts.completed),
        ));

        // Number of Failed Sub-operations (0000,1022)
        response.put(DataElement::new(
            tags::NUMBER_OF_FAILED_SUBOPERATIONS,
            VR::US,
            PrimitiveValue::from(counts.failed),
        ));

        // Number of Warning Sub-operations (0000,1023)
        response.put(DataElement::new(
            tags::NUMBER_OF_WARNING_SUBOPERATIONS,
            VR::US,
            PrimitiveValue::from(counts.warning),
        ));

        // Encode and send
        let ts = TransferSyntaxRegistry
            .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
            .ok_or_else(|| {
                DimseError::operation_failed("Implicit VR Little Endian TS not found")
            })?;

        let mut response_bytes = Vec::new();
        response
            .write_dataset_with_ts(&mut response_bytes, ts)
            .map_err(|e| {
                DimseError::operation_failed(format!("Failed to encode C-MOVE response: {}", e))
            })?;

        let pdata = dicom_ul::pdu::PDataValue {
            presentation_context_id: 1,
            value_type: dicom_ul::pdu::PDataValueType::Command,
            is_last: true,
            data: response_bytes,
        };

        association
            .send(&Pdu::PData { data: vec![pdata] })
            .await
            .map_err(|e| DimseError::network(format!("Failed to send C-MOVE response: {}", e)))?;

        Ok(())
    }

    /// Handle C-GET request (stub - not fully implemented)
    async fn handle_c_get(
        &self,
        association: &mut ServerAssociation<tokio::net::TcpStream>,
        message_id: u16,
        identifier_data: Vec<u8>,
        _presentation_context_id: u8,
    ) -> Result<()> {
        if !self.config.enable_get {
            return Err(DimseError::operation_failed("C-GET not enabled"));
        }

        debug!(
            "Handling C-GET request (message ID: {}, identifier size: {} bytes)",
            message_id,
            identifier_data.len()
        );
        warn!("C-GET operation not fully implemented - returning 'Unable to perform sub-operations' status");

        // Send failure response with "Unable to perform sub-operations" status
        // Status 0xA702 = Unable to perform sub-operations
        self.send_cget_response(
            association,
            message_id,
            0xA702,
            SubOperationCounts {
                remaining: 0,
                completed: 0,
                failed: 0,
                warning: 0,
            },
        )
        .await?;

        info!("C-GET request handled with 'not implemented' status");
        Ok(())
    }

    /// Send a C-GET response
    async fn send_cget_response(
        &self,
        association: &mut ServerAssociation<tokio::net::TcpStream>,
        message_id: u16,
        status: u16,
        counts: SubOperationCounts,
    ) -> Result<()> {
        let mut response = InMemDicomObject::new_empty();

        // Command Field (0000,0100) = 0x8010 (C-GET-RSP)
        response.put(DataElement::new(
            tags::COMMAND_FIELD,
            VR::US,
            PrimitiveValue::from(0x8010u16),
        ));

        // Message ID Being Responded To (0000,0120)
        response.put(DataElement::new(
            tags::MESSAGE_ID_BEING_RESPONDED_TO,
            VR::US,
            PrimitiveValue::from(message_id),
        ));

        // Command Data Set Type (0000,0800) = 0x0101 (no dataset)
        response.put(DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            PrimitiveValue::from(0x0101u16),
        ));

        // Status (0000,0900)
        response.put(DataElement::new(
            tags::STATUS,
            VR::US,
            PrimitiveValue::from(status),
        ));

        // Affected SOP Class UID (0000,0002) = Study Root Query/Retrieve - GET
        response.put(DataElement::new(
            tags::AFFECTED_SOP_CLASS_UID,
            VR::UI,
            PrimitiveValue::from("1.2.840.10008.5.1.4.1.2.2.3"),
        ));

        // Add sub-operation status fields
        // Number of Remaining Sub-operations (0000,1020)
        response.put(DataElement::new(
            tags::NUMBER_OF_REMAINING_SUBOPERATIONS,
            VR::US,
            PrimitiveValue::from(counts.remaining),
        ));

        // Number of Completed Sub-operations (0000,1021)
        response.put(DataElement::new(
            tags::NUMBER_OF_COMPLETED_SUBOPERATIONS,
            VR::US,
            PrimitiveValue::from(counts.completed),
        ));

        // Number of Failed Sub-operations (0000,1022)
        response.put(DataElement::new(
            tags::NUMBER_OF_FAILED_SUBOPERATIONS,
            VR::US,
            PrimitiveValue::from(counts.failed),
        ));

        // Number of Warning Sub-operations (0000,1023)
        response.put(DataElement::new(
            tags::NUMBER_OF_WARNING_SUBOPERATIONS,
            VR::US,
            PrimitiveValue::from(counts.warning),
        ));

        // Encode and send
        let ts = TransferSyntaxRegistry
            .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
            .ok_or_else(|| {
                DimseError::operation_failed("Implicit VR Little Endian TS not found")
            })?;

        let mut response_bytes = Vec::new();
        response
            .write_dataset_with_ts(&mut response_bytes, ts)
            .map_err(|e| {
                DimseError::operation_failed(format!("Failed to encode C-GET response: {}", e))
            })?;

        let pdata = dicom_ul::pdu::PDataValue {
            presentation_context_id: 1,
            value_type: dicom_ul::pdu::PDataValueType::Command,
            is_last: true,
            data: response_bytes,
        };

        association
            .send(&Pdu::PData { data: vec![pdata] })
            .await
            .map_err(|e| DimseError::network(format!("Failed to send C-GET response: {}", e)))?;

        Ok(())
    }

    /// Handle C-STORE request
    async fn handle_c_store(
        &self,
        association: &mut ServerAssociation<tokio::net::TcpStream>,
        message_id: u16,
        dataset_data: Vec<u8>,
        presentation_context_id: u8,
    ) -> Result<()> {
        if !self.config.enable_store {
            return Err(DimseError::operation_failed("C-STORE not enabled"));
        }

        debug!(
            "Handling C-STORE request (message ID: {}, dataset size: {} bytes)",
            message_id,
            dataset_data.len()
        );

        // Get the transfer syntax for this presentation context
        let ts = association
            .presentation_contexts()
            .iter()
            .find(|pc| pc.id == presentation_context_id)
            .and_then(|pc| TransferSyntaxRegistry.get(&pc.transfer_syntax))
            .ok_or_else(|| {
                DimseError::parse(format!(
                    "Transfer syntax not found for presentation context {}",
                    presentation_context_id
                ))
            })?;

        // Parse the dataset
        let cursor = std::io::Cursor::new(&dataset_data);
        let obj = InMemDicomObject::<StandardDataDictionary>::read_dataset_with_ts_cs(
            cursor,
            ts,
            SpecificCharacterSet::default(),
        )
        .map_err(|e| DimseError::parse(format!("Failed to parse C-STORE dataset: {}", e)))?;

        // Create DatasetStream
        let dataset = DatasetStream::from_object(obj);

        // Store the dataset
        match self.query_provider.store(dataset).await {
            Ok(()) => {
                // Send success response
                self.send_cstore_response(association, message_id, 0x0000, presentation_context_id)
                    .await
            }
            Err(e) => {
                error!("Failed to store dataset: {}", e);
                // Send failure response (0xC000 = Error: Cannot Understand)
                self.send_cstore_response(association, message_id, 0xC000, presentation_context_id)
                    .await
            }
        }
    }

    /// Send C-STORE response
    async fn send_cstore_response(
        &self,
        association: &mut ServerAssociation<tokio::net::TcpStream>,
        message_id: u16,
        status: u16,
        presentation_context_id: u8,
    ) -> Result<()> {
        // Build C-STORE-RSP command dataset
        let mut response = InMemDicomObject::new_empty();

        // Command Field (0000,0100) = 0x8001 (C-STORE-RSP)
        response.put(DataElement::new(
            tags::COMMAND_FIELD,
            VR::US,
            PrimitiveValue::from(0x8001u16),
        ));

        // Message ID Being Responded To (0000,0120)
        response.put(DataElement::new(
            tags::MESSAGE_ID_BEING_RESPONDED_TO,
            VR::US,
            PrimitiveValue::from(message_id),
        ));

        // Command Data Set Type (0000,0800) = 0x0101 (no dataset)
        response.put(DataElement::new(
            tags::COMMAND_DATA_SET_TYPE,
            VR::US,
            PrimitiveValue::from(0x0101u16),
        ));

        // Status (0000,0900)
        response.put(DataElement::new(
            tags::STATUS,
            VR::US,
            PrimitiveValue::from(status),
        ));

        // Encode response
        let ts = TransferSyntaxRegistry
            .get(uids::IMPLICIT_VR_LITTLE_ENDIAN)
            .ok_or_else(|| {
                DimseError::operation_failed("Implicit VR Little Endian TS not found")
            })?;

        let mut response_bytes = Vec::new();
        response
            .write_dataset_with_ts(&mut response_bytes, ts)
            .map_err(|e| {
                DimseError::operation_failed(format!("Failed to encode C-STORE response: {}", e))
            })?;

        let pdata = dicom_ul::pdu::PDataValue {
            presentation_context_id,
            value_type: dicom_ul::pdu::PDataValueType::Command,
            is_last: true,
            data: response_bytes,
        };

        association
            .send(&Pdu::PData {
                data: vec![pdata],
            })
            .await
            .map_err(|e| DimseError::network(format!("Failed to send C-STORE response: {}", e)))?;

        Ok(())
    }

    #[allow(dead_code)]
    async fn handle_dimse_request(
        &self,
        request: DimseRequest,
        router: &Arc<dyn Router>,
    ) -> Result<()> {
        let request_id = request.id;
        let _span =
            span!(Level::DEBUG, "dimse_request", id = %request_id, command = ?request.command)
                .entered();

        match request.payload {
            DimseRequestPayload::Echo => {
                debug!("Processing C-ECHO request");
                let response = if self.config.enable_echo {
                    DimseResponse::echo(request_id, true)
                } else {
                    DimseResponse::error(request_id, "C-ECHO not supported".to_string())
                };

                self.send_response(request, response, router).await?;
            }

            DimseRequestPayload::Find(ref query) => {
                debug!(
                    "Processing C-FIND request: level={}, params={:?}",
                    query.query_level, query.parameters
                );

                if !self.config.enable_find {
                    let response =
                        DimseResponse::error(request_id, "C-FIND not supported".to_string());
                    self.send_response(request, response, router).await?;
                    return Ok(());
                }

                match self
                    .query_provider
                    .find(query.query_level, &query.parameters, query.max_results)
                    .await
                {
                    Ok(datasets) => {
                        debug!("Found {} matching datasets", datasets.len());

                        // Send each dataset as a pending response
                        for (i, dataset) in datasets.iter().enumerate() {
                            let is_final = i == datasets.len() - 1;
                            let response =
                                DimseResponse::find(request_id, Some(dataset.clone()), is_final);

                            if let Some(ref stream_tx) = request.stream_tx {
                                stream_tx.send(response).await.map_err(|_| {
                                    DimseError::router("Failed to send stream response")
                                })?;
                            }
                        }

                        // Send final empty response if no datasets found
                        if datasets.is_empty() {
                            let response = DimseResponse::find(request_id, None, true);
                            self.send_response(request, response, router).await?;
                        }
                    }
                    Err(e) => {
                        let response = DimseResponse::error(request_id, e.to_string());
                        self.send_response(request, response, router).await?;
                    }
                }
            }

            DimseRequestPayload::Move(ref query) => {
                debug!(
                    "Processing C-MOVE request: level={}, dest={}",
                    query.query_level, query.destination_aet
                );

                if !self.config.enable_move {
                    let response =
                        DimseResponse::error(request_id, "C-MOVE not supported".to_string());
                    self.send_response(request, response, router).await?;
                    return Ok(());
                }

                // TODO: Implement actual C-MOVE logic
                // For now, just locate the datasets and report status
                match self
                    .query_provider
                    .locate(query.query_level, &query.parameters)
                    .await
                {
                    Ok(datasets) => {
                        let total = datasets.len() as u32;
                        debug!("Located {} datasets for move", total);

                        // Send final status response
                        let response = DimseResponse::move_response(
                            request_id, None, 0,     // remaining
                            total, // completed
                            0,     // failed
                            0,     // warning
                            true,  // is_final
                        );
                        self.send_response(request, response, router).await?;
                    }
                    Err(e) => {
                        let response = DimseResponse::error(request_id, e.to_string());
                        self.send_response(request, response, router).await?;
                    }
                }
            }

            DimseRequestPayload::Get(ref query) => {
                debug!("Processing C-GET request: level={}", query.query_level);

                if !self.config.enable_get {
                    let response =
                        DimseResponse::error(request_id, "C-GET not supported".to_string());
                    self.send_response(request, response, router).await?;
                    return Ok(());
                }

                match self
                    .query_provider
                    .get(query.query_level, &query.parameters)
                    .await
                {
                    Ok(datasets) => {
                        let total = datasets.len() as u32;
                        debug!("Retrieved {} datasets for get", total);

                        // Send each dataset as a pending response
                        for (i, dataset) in datasets.iter().enumerate() {
                            let is_final = i == datasets.len() - 1;
                            let remaining = total - (i as u32) - 1;
                            let completed = (i as u32) + 1;

                            let response = DimseResponse::get_response(
                                request_id,
                                Some(dataset.clone()),
                                remaining,
                                completed,
                                0, // failed
                                0, // warning
                                is_final,
                            );

                            if let Some(ref stream_tx) = request.stream_tx {
                                stream_tx.send(response).await.map_err(|_| {
                                    DimseError::router("Failed to send stream response")
                                })?;
                            }
                        }

                        // Send final empty response if no datasets found
                        if datasets.is_empty() {
                            let response =
                                DimseResponse::get_response(request_id, None, 0, 0, 0, 0, true);
                            self.send_response(request, response, router).await?;
                        }
                    }
                    Err(e) => {
                        let response = DimseResponse::error(request_id, e.to_string());
                        self.send_response(request, response, router).await?;
                    }
                }
            }

            DimseRequestPayload::Store(ref dataset) => {
                debug!("Processing C-STORE request");

                match self.query_provider.store(dataset.clone()).await {
                    Ok(()) => {
                        let response = DimseResponse::store(request_id, true);
                        self.send_response(request, response, router).await?;
                    }
                    Err(e) => {
                        let response = DimseResponse::error(request_id, e.to_string());
                        self.send_response(request, response, router).await?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Send a response back through the appropriate channel
    #[allow(dead_code)]
    async fn send_response(
        &self,
        request: DimseRequest,
        response: DimseResponse,
        router: &Arc<dyn Router>,
    ) -> Result<()> {
        if let Some(response_tx) = request.response_tx {
            response_tx
                .send(response)
                .map_err(|_| DimseError::router("Failed to send response"))?;
        } else {
            router.send_response(response).await?;
        }
        Ok(())
    }
}

/// Default query provider implementation (for testing)
pub struct DefaultQueryProvider {
    storage_dir: std::path::PathBuf,
}

impl DefaultQueryProvider {
    pub fn new(storage_dir: std::path::PathBuf) -> Self {
        Self { storage_dir }
    }
}

#[async_trait]
impl QueryProvider for DefaultQueryProvider {
    async fn find(
        &self,
        _query_level: QueryLevel,
        _parameters: &std::collections::HashMap<String, String>,
        _max_results: u32,
    ) -> Result<Vec<DatasetStream>> {
        // TODO: Implement actual query logic
        warn!("DefaultQueryProvider::find not yet implemented");
        Ok(vec![])
    }

    async fn locate(
        &self,
        _query_level: QueryLevel,
        _parameters: &std::collections::HashMap<String, String>,
    ) -> Result<Vec<DatasetStream>> {
        // TODO: Implement actual locate logic
        warn!("DefaultQueryProvider::locate not yet implemented");
        Ok(vec![])
    }

    async fn get(
        &self,
        _query_level: QueryLevel,
        _parameters: &std::collections::HashMap<String, String>,
    ) -> Result<Vec<DatasetStream>> {
        // TODO: Implement actual get logic
        warn!("DefaultQueryProvider::get not yet implemented");
        Ok(vec![])
    }

    async fn store(&self, dataset: DatasetStream) -> Result<()> {
        // Store the dataset to the storage directory
        let temp_file = dataset.to_temp_file(&self.storage_dir).await?;
        info!("Stored dataset to {}", temp_file.display());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[tokio::test]
    async fn test_scp_creation() {
        let config = DimseConfig {
            local_aet: "TEST_SCP".to_string(),
            bind_addr: std::net::IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            port: 0, // Use any available port
            ..Default::default()
        };

        let temp_dir = tempfile::tempdir().unwrap();
        let query_provider = Arc::new(DefaultQueryProvider::new(temp_dir.path().to_path_buf()));

        let scp = DimseScp::new(config, query_provider);
        assert_eq!(scp.config.local_aet, "TEST_SCP");
    }

    #[test]
    fn test_default_query_provider() {
        let temp_dir = tempfile::tempdir().unwrap();
        let _provider = DefaultQueryProvider::new(temp_dir.path().to_path_buf());
        // Basic creation test
    }
}
