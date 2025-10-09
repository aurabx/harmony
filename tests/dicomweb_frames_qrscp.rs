use axum::body::Body;
use axum::http::{Request, StatusCode};
use harmony::config::config::{Config, ConfigError};
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

fn load_config_from_str(toml: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(toml).expect("TOML parse error");
    config.validate()?;
    Ok(config)
}

#[tokio::test]
async fn dicomweb_frames_with_dcmqrscp() {
    // Require dcmtk tools
    for bin in ["dcmqrscp", "storescu"].iter() {
        if std::process::Command::new(bin).arg("--version").output().is_err() {
            eprintln!("Skipping frames test: {} not found", bin);
            return;
        }
    }

    // Pick free port
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    // Prepare dcmqrscp config
    let base = PathBuf::from("./tmp/qrscp_frames");
    let dbdir = base.join("qrdb");
    std::fs::create_dir_all(&dbdir).expect("mkdir qr db");
    let cfg_path = base.join("dcmqrscp.cfg");
    let abs_db = std::fs::canonicalize(&dbdir).unwrap_or_else(|_| std::env::current_dir().unwrap().join(&dbdir));
    let cfg = format!(
        "# dcmqrscp cfg\nMaxPDUSize = 16384\nMaxAssociations = 16\n\nHostTable BEGIN\nHostTable END\n\nVendorTable BEGIN\nVendorTable END\n\nAETable BEGIN\nQR_SCP  {db}  RW  (9, 1024mb)  ANY\nAETable END\n",
        db = abs_db.to_string_lossy()
    );
    std::fs::create_dir_all(&base).expect("mkdir cfg dir");
    std::fs::write(&cfg_path, cfg).expect("write cfg");

    // Start dcmqrscp (quiet by default; enable verbose with HARMONY_TEST_VERBOSE_DCMTK=1)
    let verbose = std::env::var("HARMONY_TEST_VERBOSE_DCMTK").ok().as_deref() == Some("1");
    let mut dcmqr = tokio::process::Command::new("dcmqrscp");
    if verbose { dcmqr.arg("-d"); }
    let mut dcmqr = dcmqr
        .arg("-c")
        .arg(&cfg_path)
        .arg(port.to_string());
    if !verbose { dcmqr.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()); }
    let mut qr_child = dcmqr
        .kill_on_drop(true)
        .spawn()
        .expect("spawn dcmqrscp");

    for _ in 0..60 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // Build a tiny 1x1 MONOCHROME2 image dataset with pixel data
    let mkuid = |suf: &str| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        format!("1.2.826.0.1.3680043.10.5432.{}.{}.{}", suf, now.as_secs(), now.subsec_nanos())
    };
    let study_uid = mkuid("study");
    let series_uid = mkuid("series");
    let sop_uid = mkuid("sop");

    use dicom_core::header::{DataElement, Tag};
    use dicom_core::value::PrimitiveValue;
    use dicom_core::VR;
    use dicom_object::{InMemDicomObject, DefaultDicomObject};

    let mut obj = InMemDicomObject::new_empty();
    let put = |o: &mut InMemDicomObject, tag: Tag, vr: VR, val: PrimitiveValue| {
        o.put(DataElement::new(tag, vr, val));
    };

    put(&mut obj, Tag(0x0008, 0x0016), VR::UI, PrimitiveValue::from("1.2.840.10008.5.1.4.1.1.7")); // SC
    put(&mut obj, Tag(0x0008, 0x0018), VR::UI, PrimitiveValue::from(sop_uid.as_str()));
    put(&mut obj, Tag(0x0020, 0x000D), VR::UI, PrimitiveValue::from(study_uid.as_str()));
    put(&mut obj, Tag(0x0020, 0x000E), VR::UI, PrimitiveValue::from(series_uid.as_str()));
    put(&mut obj, Tag(0x0008, 0x0060), VR::CS, PrimitiveValue::from("OT"));

    // Image attributes
    put(&mut obj, Tag(0x0028, 0x0002), VR::US, PrimitiveValue::from(1u16)); // SamplesPerPixel
    put(&mut obj, Tag(0x0028, 0x0004), VR::CS, PrimitiveValue::from("MONOCHROME2"));
    put(&mut obj, Tag(0x0028, 0x0010), VR::US, PrimitiveValue::from(1u16)); // Rows
    put(&mut obj, Tag(0x0028, 0x0011), VR::US, PrimitiveValue::from(1u16)); // Columns
    put(&mut obj, Tag(0x0028, 0x0100), VR::US, PrimitiveValue::from(8u16)); // BitsAllocated
    put(&mut obj, Tag(0x0028, 0x0101), VR::US, PrimitiveValue::from(8u16)); // BitsStored
    put(&mut obj, Tag(0x0028, 0x0102), VR::US, PrimitiveValue::from(7u16)); // HighBit
    put(&mut obj, Tag(0x0028, 0x0103), VR::US, PrimitiveValue::from(0u16)); // PixelRepresentation

    // Pixel Data: single 8-bit pixel with value 128
    put(&mut obj, Tag(0x7FE0, 0x0010), VR::OB, PrimitiveValue::U8(vec![128u8].into()));

    let dicom_path = base.join("seed_frame.dcm");
    dicom_json_tool::write_part10(&dicom_path, &obj).expect("write dicom");

    // Store into QR
    let mut st = tokio::process::Command::new("storescu");
    let mut st = st
        .arg("--aetitle").arg("HARMONY_SCU")
        .arg("--call").arg("QR_SCP")
        .arg("127.0.0.1").arg(port.to_string())
        .arg(&dicom_path);
    if !verbose { st.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()); }
    let status = st.status().await.expect("run storescu");
    if !status.success() {
        eprintln!("storescu failed; skipping");
        let _ = qr_child.kill().await;
        return;
    }

    // Build config with DICOMweb endpoint, middlewares, and DICOM backend
    let toml = format!(r#"
        [proxy]
        id = "dicomweb-frames-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8085

        [pipelines.bridge]
        description = "DICOMweb -> DICOM bridge with frames"
        networks = ["default"]
        endpoints = ["dicomweb"]
        middleware = ["dicomweb_to_dicom", "dicom_to_dicomweb"]
        backends = ["dicom_pacs"]

        [endpoints.dicomweb]
        service = "dicomweb"
        [endpoints.dicomweb.options]
        path_prefix = "/dicomweb"

        [backends.dicom_pacs]
        service = "dicom"
        [backends.dicom_pacs.options]
        aet = "QR_SCP"
        host = "127.0.0.1"
        port = {port}
        local_aet = "HARMONY_SCU"

        [services.dicomweb]
        module = ""
        [services.dicom]
        module = ""

        [middleware_types.dicomweb_to_dicom]
        module = ""
        [middleware_types.dicom_to_dicomweb]
        module = ""
    "#, port = port);

    let cfg: Config = load_config_from_str(&toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Issue DICOMweb frames request
    let url = format!(
        "/dicomweb/studies/{}/series/{}/instances/{}/frames/1",
        study_uid, series_uid, sop_uid
    );
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&url)
                .header("accept", "image/jpeg")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers().clone();
    let content_type = headers.get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("");
    assert!(content_type.starts_with("image/jpeg"), "unexpected content-type: {}", content_type);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(!body.is_empty(), "Expected non-empty JPEG body");

    let _ = qr_child.kill().await;
}

#[tokio::test]
async fn dicomweb_multiframes_with_dcmqrscp() {
    // Require dcmtk tools
    for bin in ["dcmqrscp", "storescu"].iter() {
        if std::process::Command::new(bin).arg("--version").output().is_err() {
            eprintln!("Skipping multi-frames test: {} not found", bin);
            return;
        }
    }

    // Pick free port
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    // Prepare dcmqrscp config
    let base = PathBuf::from("./tmp/qrscp_multiframes");
    let dbdir = base.join("qrdb");
    std::fs::create_dir_all(&dbdir).expect("mkdir qr db");
    let cfg_path = base.join("dcmqrscp.cfg");
    let abs_db = std::fs::canonicalize(&dbdir).unwrap_or_else(|_| std::env::current_dir().unwrap().join(&dbdir));
    let cfg = format!(
        "# dcmqrscp cfg\nMaxPDUSize = 16384\nMaxAssociations = 16\n\nHostTable BEGIN\nHostTable END\n\nVendorTable BEGIN\nVendorTable END\n\nAETable BEGIN\nQR_SCP  {db}  RW  (9, 1024mb)  ANY\nAETable END\n",
        db = abs_db.to_string_lossy()
    );
    std::fs::create_dir_all(&base).expect("mkdir cfg dir");
    std::fs::write(&cfg_path, cfg).expect("write cfg");

    // Start dcmqrscp (quiet by default; enable verbose with HARMONY_TEST_VERBOSE_DCMTK=1)
    let verbose = std::env::var("HARMONY_TEST_VERBOSE_DCMTK").ok().as_deref() == Some("1");
    let mut dcmqr = tokio::process::Command::new("dcmqrscp");
    if verbose { dcmqr.arg("-d"); }
    let mut dcmqr = dcmqr
        .arg("-c")
        .arg(&cfg_path)
        .arg(port.to_string());
    if !verbose { dcmqr.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()); }
    let mut qr_child = dcmqr
        .kill_on_drop(true)
        .spawn()
        .expect("spawn dcmqrscp");

    for _ in 0..60 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // Build a 1x1 MONOCHROME2, NumberOfFrames=2 image
    let mkuid = |suf: &str| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        format!("1.2.826.0.1.3680043.10.5432.{}.{}.{}", suf, now.as_secs(), now.subsec_nanos())
    };
    let study_uid = mkuid("study");
    let series_uid = mkuid("series");
    let sop_uid = mkuid("sop");

    use dicom_core::header::{DataElement, Tag};
    use dicom_core::value::PrimitiveValue;
    use dicom_core::VR;
    use dicom_object::InMemDicomObject;

    let mut obj = InMemDicomObject::new_empty();
    let put = |o: &mut InMemDicomObject, tag: Tag, vr: VR, val: PrimitiveValue| {
        o.put(DataElement::new(tag, vr, val));
    };

    // SC with Multi-frame
    put(&mut obj, Tag(0x0008, 0x0016), VR::UI, PrimitiveValue::from("1.2.840.10008.5.1.4.1.1.7"));
    put(&mut obj, Tag(0x0008, 0x0018), VR::UI, PrimitiveValue::from(sop_uid.as_str()));
    put(&mut obj, Tag(0x0020, 0x000D), VR::UI, PrimitiveValue::from(study_uid.as_str()));
    put(&mut obj, Tag(0x0020, 0x000E), VR::UI, PrimitiveValue::from(series_uid.as_str()));
    put(&mut obj, Tag(0x0008, 0x0060), VR::CS, PrimitiveValue::from("OT"));
    put(&mut obj, Tag(0x0028, 0x0002), VR::US, PrimitiveValue::from(1u16));
    put(&mut obj, Tag(0x0028, 0x0004), VR::CS, PrimitiveValue::from("MONOCHROME2"));
    put(&mut obj, Tag(0x0028, 0x0008), VR::IS, PrimitiveValue::from("2")); // NumberOfFrames
    put(&mut obj, Tag(0x0028, 0x0010), VR::US, PrimitiveValue::from(1u16)); // Rows
    put(&mut obj, Tag(0x0028, 0x0011), VR::US, PrimitiveValue::from(1u16)); // Columns
    put(&mut obj, Tag(0x0028, 0x0100), VR::US, PrimitiveValue::from(8u16));
    put(&mut obj, Tag(0x0028, 0x0101), VR::US, PrimitiveValue::from(8u16));
    put(&mut obj, Tag(0x0028, 0x0102), VR::US, PrimitiveValue::from(7u16));
    put(&mut obj, Tag(0x0028, 0x0103), VR::US, PrimitiveValue::from(0u16));
    put(&mut obj, Tag(0x7FE0, 0x0010), VR::OB, PrimitiveValue::U8(vec![120u8, 80u8].into()));

    let dicom_path = base.join("seed_multiframe.dcm");
    dicom_json_tool::write_part10(&dicom_path, &obj).expect("write dicom");

    // Store into QR
    let mut st = tokio::process::Command::new("storescu");
    let mut st = st
        .arg("--aetitle").arg("HARMONY_SCU")
        .arg("--call").arg("QR_SCP")
        .arg("127.0.0.1").arg(port.to_string())
        .arg(&dicom_path);
    if !verbose { st.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()); }
    let status = st.status().await.expect("run storescu");
    if !status.success() {
        eprintln!("storescu failed; skipping");
        let _ = qr_child.kill().await;
        return;
    }

    // Build pipeline config
    let toml = format!(r#"
        [proxy]
        id = "dicomweb-multiframes-test"
        log_level = "info"
        store_dir = "./tmp"

        [network.default]
        enable_wireguard = false
        interface = "wg0"

        [network.default.http]
        bind_address = "127.0.0.1"
        bind_port = 8086

        [pipelines.bridge]
        description = "DICOMweb -> DICOM bridge with multiframes"
        networks = ["default"]
        endpoints = ["dicomweb"]
        middleware = ["dicomweb_to_dicom", "dicom_to_dicomweb"]
        backends = ["dicom_pacs"]

        [endpoints.dicomweb]
        service = "dicomweb"
        [endpoints.dicomweb.options]
        path_prefix = "/dicomweb"

        [backends.dicom_pacs]
        service = "dicom"
        [backends.dicom_pacs.options]
        aet = "QR_SCP"
        host = "127.0.0.1"
        port = {port}
        local_aet = "HARMONY_SCU"

        [services.dicomweb]
        module = ""
        [services.dicom]
        module = ""

        [middleware_types.dicomweb_to_dicom]
        module = ""
        [middleware_types.dicom_to_dicomweb]
        module = ""
    "#, port = port);

    let cfg: Config = load_config_from_str(&toml).expect("valid config");
    let app = harmony::router::build_network_router(Arc::new(cfg), "default").await;

    // Request frames 1 and 2
    let url = format!(
        "/dicomweb/studies/{}/series/{}/instances/{}/frames/1,2",
        study_uid, series_uid, sop_uid
    );
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&url)
                .header("accept", "image/jpeg")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router handled request");

    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(content_type.starts_with("multipart/related"), "unexpected content-type: {}", content_type);

    // Extract boundary from header
    let boundary = content_type
        .split(';')
        .filter_map(|p| p.trim().strip_prefix("boundary="))
        .map(|s| s.trim().trim_matches('"'))
        .next()
        .unwrap_or("")
        .to_string();
    assert!(!boundary.is_empty(), "Missing boundary in content-type");

    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    // Count boundary occurrences
    let marker = format!("--{}\r\n", boundary);
    let count = body.windows(marker.len()).filter(|w| *w == marker.as_bytes()).count();
    assert!(count >= 2, "Expected at least 2 parts, found {}", count);

    let _ = qr_child.kill().await;
}
