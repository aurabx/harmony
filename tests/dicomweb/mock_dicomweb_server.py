#!/usr/bin/env python3
"""
Mock DICOMweb server for testing dicom_to_dicomweb middleware.
Supports QIDO-RS, STOW-RS, and WADO-RS endpoints.
"""

import json
import sys
import logging
from http.server import HTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse, parse_qs
from io import BytesIO

# Configure logging
logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(message)s')
logger = logging.getLogger(__name__)

# Mock data store
studies = {}
stored_instances = []


class MockDICOMwebHandler(BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        logger.info("%s - %s" % (self.address_string(), format % args))

    def do_GET(self):
        """Handle QIDO-RS (query) and WADO-RS (retrieve) requests"""
        parsed = urlparse(self.path)
        path = parsed.path
        params = parse_qs(parsed.query)

        logger.info(f"GET {path} with params: {params}")

        # QIDO-RS: Query for studies
        if path == "/studies" or path == "/dicom-web/studies":
            self.handle_qido_studies(params)
        # QIDO-RS: Query for series in a study
        elif "/studies/" in path and "/series" in path and "/instances" not in path:
            self.handle_qido_series(path, params)
        # QIDO-RS: Query for instances in a series
        elif "/studies/" in path and "/series/" in path and "/instances" in path:
            self.handle_qido_instances(path, params)
        # WADO-RS: Retrieve study/series/instance
        elif "/studies/" in path and not "/metadata" in path:
            self.handle_wado_retrieve(path)
        else:
            self.send_error(404, "Endpoint not found")

    def do_POST(self):
        """Handle STOW-RS (store) requests"""
        parsed = urlparse(self.path)
        path = parsed.path

        logger.info(f"POST {path}")

        # STOW-RS: Store instances
        if path == "/studies" or path == "/dicom-web/studies":
            self.handle_stow_store()
        else:
            self.send_error(404, "Endpoint not found")

    def handle_qido_studies(self, params):
        """Handle QIDO-RS study-level query"""
        # Return mock study data
        results = [
            {
                "0020000D": {"vr": "UI", "Value": ["1.2.840.113619.2.55.3.12345"]},
                "00100010": {"vr": "PN", "Value": [{"Alphabetic": "Test^Patient"}]},
                "00100020": {"vr": "LO", "Value": ["TEST123"]},
                "00080020": {"vr": "DA", "Value": ["20240101"]},
                "00080030": {"vr": "TM", "Value": ["120000"]},
                "00080050": {"vr": "SH", "Value": ["ACC001"]},
                "00080060": {"vr": "CS", "Value": ["CT"]},
                "00081030": {"vr": "LO", "Value": ["Test Study"]},
            }
        ]

        # Filter by query parameters
        if "00100020" in params:  # PatientID
            patient_id = params["00100020"][0]
            results = [r for r in results if r["00100020"]["Value"][0] == patient_id]

        self.send_json_response(200, results)

    def handle_qido_series(self, path, params):
        """Handle QIDO-RS series-level query"""
        # Extract study UID from path
        parts = path.split("/")
        study_idx = parts.index("studies") + 1 if "studies" in parts else -1
        
        results = [
            {
                "0020000D": {"vr": "UI", "Value": ["1.2.840.113619.2.55.3.12345"]},
                "0020000E": {"vr": "UI", "Value": ["1.2.840.113619.2.55.3.12345.1"]},
                "00080060": {"vr": "CS", "Value": ["CT"]},
                "0008103E": {"vr": "LO", "Value": ["Test Series"]},
                "00200011": {"vr": "IS", "Value": ["1"]},
            }
        ]

        self.send_json_response(200, results)

    def handle_qido_instances(self, path, params):
        """Handle QIDO-RS instance-level query"""
        results = [
            {
                "0020000D": {"vr": "UI", "Value": ["1.2.840.113619.2.55.3.12345"]},
                "0020000E": {"vr": "UI", "Value": ["1.2.840.113619.2.55.3.12345.1"]},
                "00080018": {"vr": "UI", "Value": ["1.2.840.113619.2.55.3.12345.1.1"]},
                "00200013": {"vr": "IS", "Value": ["1"]},
            }
        ]

        self.send_json_response(200, results)

    def handle_wado_retrieve(self, path):
        """Handle WADO-RS retrieve request"""
        # Check Accept header
        accept = self.headers.get("Accept", "")
        
        if "application/dicom" in accept or "multipart/related" in accept:
            # Return mock DICOM data
            mock_dicom = b"FAKE_DICOM_CONTENT_FOR_TESTING"
            
            if "multipart/related" in accept:
                # Return as multipart
                boundary = "boundary_mock_wado"
                body = (
                    f"--{boundary}\r\n"
                    "Content-Type: application/dicom\r\n\r\n"
                ).encode() + mock_dicom + f"\r\n--{boundary}--\r\n".encode()
                
                self.send_response(200)
                self.send_header("Content-Type", f'multipart/related; type="application/dicom"; boundary={boundary}')
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            else:
                # Return as single DICOM object
                self.send_response(200)
                self.send_header("Content-Type", "application/dicom")
                self.send_header("Content-Length", str(len(mock_dicom)))
                self.end_headers()
                self.wfile.write(mock_dicom)
        else:
            self.send_error(406, "Not Acceptable")

    def handle_stow_store(self):
        """Handle STOW-RS store request"""
        content_type = self.headers.get("Content-Type", "")
        content_length = int(self.headers.get("Content-Length", 0))
        
        if content_length > 0:
            body = self.rfile.read(content_length)
            logger.info(f"Received STOW request with {len(body)} bytes")
            
            # Store the instance
            stored_instances.append({
                "content_type": content_type,
                "size": len(body),
                "data": body
            })
            
            # Return success response
            response = {
                "00081190": {"vr": "UR", "Value": ["http://mock/studies/1.2.3"]},
                "00081198": {
                    "vr": "SQ",
                    "Value": [
                        {
                            "00081150": {"vr": "UI", "Value": ["1.2.840.10008.5.1.4.1.1.2"]},
                            "00081155": {"vr": "UI", "Value": ["1.2.840.113619.2.55.3.12345.1.1"]},
                        }
                    ]
                }
            }
            self.send_json_response(200, response)
        else:
            self.send_error(400, "No content")

    def send_json_response(self, status, data):
        """Send JSON response"""
        body = json.dumps(data).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/dicom+json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def run_server(port=8042):
    server_address = ("127.0.0.1", port)
    httpd = HTTPServer(server_address, MockDICOMwebHandler)
    logger.info(f"Mock DICOMweb server running on http://127.0.0.1:{port}")
    logger.info("Endpoints available:")
    logger.info("  QIDO-RS: GET /studies, /studies/{uid}/series, /studies/{uid}/series/{uid}/instances")
    logger.info("  WADO-RS: GET /studies/{uid}, /studies/{uid}/series/{uid}, /studies/{uid}/series/{uid}/instances/{uid}")
    logger.info("  STOW-RS: POST /studies")
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        logger.info("Shutting down mock server")
        httpd.shutdown()


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8042
    run_server(port)
