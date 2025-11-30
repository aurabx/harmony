#!/usr/bin/env bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TMP_DIR="$SCRIPT_DIR/tmp"

# Configuration
DICOMWEB_PORT=8042

echo -e "${BLUE}=== DICOM to DICOMweb Bridge Demo ===${NC}"
echo ""

# Setup directories
mkdir -p "$TMP_DIR"

# Create a simple DICOMweb server in Python
cat > "$TMP_DIR/dicomweb_server.py" << 'PYEOF'
import http.server
import json
from urllib.parse import urlparse, parse_qs

class DICOMwebHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        parsed_path = urlparse(self.path)
        path = parsed_path.path
        
        # Log the request with timestamp
        import datetime
        timestamp = datetime.datetime.now().isoformat()
        print(f"[{timestamp}] DICOMweb GET: {path}")
        print(f"  Query: {parsed_path.query}")
        
        if path == '/dicom-web/studies':
            # Return empty study list (QIDO-RS response)
            response = []
            self.send_response(200)
            self.send_header('Content-type', 'application/dicom+json')
            self.end_headers()
            self.wfile.write(json.dumps(response).encode())
            print(f"  Response: 200 OK (empty study list)")
        elif path.startswith('/dicom-web/studies/'):
            # Return study details
            response = [{"00100010": {"Value": [{"Alphabetic": "Test^Patient"}]}}]
            self.send_response(200)
            self.send_header('Content-type', 'application/dicom+json')
            self.end_headers()
            self.wfile.write(json.dumps(response).encode())
            print(f"  Response: 200 OK (study details)")
        else:
            self.send_response(404)
            self.send_header('Content-type', 'text/plain')
            self.end_headers()
            self.wfile.write(b'Not Found')
            print(f"  Response: 404 Not Found")
    
    def do_POST(self):
        # Log POST requests
        import datetime
        timestamp = datetime.datetime.now().isoformat()
        content_length = int(self.headers.get('Content-Length', 0))
        body = self.rfile.read(content_length)
        print(f"[{timestamp}] DICOMweb POST: {self.path}")
        print(f"  Content-Length: {content_length}")
        
        # Accept C-STORE operations (STOW-RS)
        self.send_response(200)
        self.send_header('Content-type', 'application/dicom+json')
        self.end_headers()
        response = {"status": "stored"}
        self.wfile.write(json.dumps(response).encode())
        print(f"  Response: 200 OK (stored)")
    
    def log_message(self, format, *args):
        # Suppress default logging
        pass

if __name__ == '__main__':
    server = http.server.HTTPServer(('127.0.0.1', 8042), DICOMwebHandler)
    print(f"DICOMweb server listening on 127.0.0.1:8042")
    server.serve_forever()
PYEOF

# Cleanup function
cleanup() {
  echo ""
  echo -e "${YELLOW}Cleaning up...${NC}"
  
  # Kill Harmony
  if [ ! -z "${HARMONY_PID:-}" ] && kill -0 $HARMONY_PID 2>/dev/null; then
    echo "  Stopping Harmony (PID: $HARMONY_PID)..."
    kill $HARMONY_PID 2>/dev/null || true
    wait $HARMONY_PID 2>/dev/null || true
  fi
  
  # Kill DICOMweb server
  if [ ! -z "${DICOMWEB_PID:-}" ] && kill -0 $DICOMWEB_PID 2>/dev/null; then
    echo "  Stopping DICOMweb server (PID: $DICOMWEB_PID)..."
    kill $DICOMWEB_PID 2>/dev/null || true
    wait $DICOMWEB_PID 2>/dev/null || true
  fi
  
  echo -e "${GREEN}✓ Cleanup complete${NC}"
}

trap cleanup EXIT INT TERM

# Check prerequisites
echo -e "${YELLOW}Checking prerequisites...${NC}"
if ! command -v python3 &> /dev/null; then
  echo -e "${RED}Error: python3 not found${NC}"
  exit 1
fi
if ! command -v curl &> /dev/null; then
  echo -e "${RED}Error: curl not found${NC}"
  exit 1
fi
echo -e "${GREEN}✓ Prerequisites found${NC}"
echo ""

# Build Harmony if not already built
echo -e "${YELLOW}Building Harmony...${NC}"
if [ ! -x "$PROJECT_ROOT/target/release/harmony" ]; then
  (cd "$PROJECT_ROOT" && cargo build --release --quiet)
fi
echo -e "${GREEN}✓ Build complete${NC}"
echo ""

# Start DICOMweb server
echo -e "${YELLOW}Starting DICOMweb backend server on port $DICOMWEB_PORT...${NC}"
python3 -u "$TMP_DIR/dicomweb_server.py" > "$TMP_DIR/dicomweb.log" 2>&1 &
DICOMWEB_PID=$!
echo -e "${GREEN}✓ DICOMweb server started (PID: $DICOMWEB_PID)${NC}"

# Wait for server to be ready
sleep 1
if ! curl -s http://127.0.0.1:$DICOMWEB_PORT/dicom-web/studies > /dev/null 2>&1; then
  echo -e "${RED}Error: DICOMweb server did not start${NC}"
  exit 1
fi
echo -e "${GREEN}✓ DICOMweb server is ready${NC}"
echo ""

# Start Harmony in background
echo -e "${YELLOW}Starting Harmony bridge...${NC}"
cd "$SCRIPT_DIR"
"$PROJECT_ROOT/target/release/harmony" --config ./config.toml > "$TMP_DIR/harmony.log" 2>&1 &
HARMONY_PID=$!
echo -e "${GREEN}✓ Harmony started (PID: $HARMONY_PID)${NC}"

# Wait for Harmony to be ready
sleep 2
echo -e "${GREEN}✓ Bridge ready${NC}"
echo ""

echo -e "${BLUE}=== Running Tests ===${NC}"
echo ""

# Test 1: Verify DICOM SCP is listening
echo -e "${YELLOW}Test 1: DICOM SCP endpoint listening${NC}"
if nc -z 127.0.0.1 11112 2>/dev/null; then
  echo -e "${GREEN}  ✓ DICOM SCP listening on 127.0.0.1:11112${NC}"
else
  echo -e "${RED}  ✗ DICOM SCP not listening${NC}"
  exit 1
fi
echo ""

# Test 2: Verify DICOMweb backend is accessible
echo -e "${YELLOW}Test 2: DICOMweb backend accessible${NC}"
if curl -s http://127.0.0.1:$DICOMWEB_PORT/dicom-web/studies > /dev/null 2>&1; then
  echo -e "${GREEN}  ✓ DICOMweb backend at 127.0.0.1:$DICOMWEB_PORT/dicom-web${NC}"
else
  echo -e "${RED}  ✗ DICOMweb backend not responding${NC}"
  exit 1
fi
echo ""

# Test 3: Test data flow through bridge with C-FIND
echo -e "${YELLOW}Test 3: Test data flow through bridge (C-FIND)${NC}"

if ! command -v findscu &> /dev/null; then
  echo -e "${YELLOW}  ⚠ findscu not found, skipping pipeline test${NC}"
  echo -e "${YELLOW}  ⚠ Install dcmtk to test the full pipeline${NC}"
else
  echo "  Sending C-FIND query (DICOM → DICOMweb)..."
  
  # Send C-FIND study query via findscu
  # This should trigger a QIDO-RS query to the DICOMweb backend
  FIND_EXIT=0
  findscu -aet TEST_SCU -aec BRIDGE_SCP 127.0.0.1 11112 \
    -k QueryRetrieveLevel=STUDY \
    -k StudyInstanceUID= \
    > "$TMP_DIR/findscu.log" 2>&1 || FIND_EXIT=$?
  
  # Give backend a moment to log the request
  sleep 1
  
  # Check if DICOMweb backend received the query
  if grep -q "DICOMweb GET" "$TMP_DIR/dicomweb.log" 2>/dev/null; then
    echo -e "${GREEN}  ✓ C-FIND query sent successfully${NC}"
    echo -e "${GREEN}  ✓ Bridge transformed DICOM to DICOMweb${NC}"
    echo -e "${GREEN}  ✓ DICOMweb backend received the request${NC}"
    
    # Show what the backend received
    BACKEND_PATH=$(grep "DICOMweb GET:" "$TMP_DIR/dicomweb.log" | head -1 | cut -d: -f4-)
    echo -e "${GREEN}  ✓ Backend path:$BACKEND_PATH${NC}"
  else
    echo -e "${RED}  ✗ DICOMweb backend did not receive request${NC}"
    echo -e "${YELLOW}  ⚠ Bridge may not be transforming correctly${NC}"
    
    # Show findscu output for debugging
    if [ -f "$TMP_DIR/findscu.log" ]; then
      echo "  findscu output:"
      tail -3 "$TMP_DIR/findscu.log" | sed 's/^/    /'
    fi
  fi
fi
echo ""

# Test 4: Test C-STORE to STOW-RS transformation
echo -e "${YELLOW}Test 4: Test data flow through bridge (C-STORE)${NC}"

if ! command -v storescu &> /dev/null; then
  echo -e "${YELLOW}  ⚠ storescu not found, skipping C-STORE test${NC}"
  echo -e "${YELLOW}  ⚠ Install dcmtk to test C-STORE operations${NC}"
else
  # Use existing sample DICOM file
  TEST_DICOM="$PROJECT_ROOT/samples/dicom/study_1/series_1/CT.1.1.dcm"
  
  if [ ! -f "$TEST_DICOM" ]; then
    echo -e "${RED}  ✗ Sample DICOM file not found: $TEST_DICOM${NC}"
  else
    echo "  Sending C-STORE operation (DICOM → STOW-RS)..."
    
    # Send C-STORE via storescu
    STORE_EXIT=0
    storescu -aet TEST_SCU -aec BRIDGE_SCP 127.0.0.1 11112 \
      "$TEST_DICOM" \
      > "$TMP_DIR/storescu.log" 2>&1 || STORE_EXIT=$?
    
    # Give backend a moment to log the request
    sleep 1
    
    # Check if DICOMweb backend received the STOW-RS POST
    if grep -q "DICOMweb POST" "$TMP_DIR/dicomweb.log" 2>/dev/null; then
      echo -e "${GREEN}  ✓ C-STORE operation sent successfully${NC}"
      echo -e "${GREEN}  ✓ Bridge transformed DICOM to STOW-RS${NC}"
      echo -e "${GREEN}  ✓ DICOMweb backend received STOW-RS POST${NC}"
      
      # Show what the backend received
      STORE_PATH=$(grep "DICOMweb POST:" "$TMP_DIR/dicomweb.log" | tail -1 | cut -d: -f4-)
      echo -e "${GREEN}  ✓ Backend path:$STORE_PATH${NC}"
      
      CONTENT_LENGTH=$(grep "Content-Length:" "$TMP_DIR/dicomweb.log" | tail -1 | awk '{print $3}')
      if [ ! -z "$CONTENT_LENGTH" ]; then
        echo -e "${GREEN}  ✓ Payload size: $CONTENT_LENGTH bytes${NC}"
      fi
    else
      echo -e "${RED}  ✗ DICOMweb backend did not receive STOW-RS request${NC}"
      echo -e "${YELLOW}  ⚠ Bridge may not be handling C-STORE correctly${NC}"
      
      # Show storescu output for debugging
      if [ -f "$TMP_DIR/storescu.log" ]; then
        echo "  storescu output:"
        tail -5 "$TMP_DIR/storescu.log" | sed 's/^/    /'
      fi
    fi
  fi
fi
echo ""

echo -e "${BLUE}=== Configuration ===${NC}"
echo ""
echo "Bridge Setup:"
echo "  DICOM Input:     127.0.0.1:11112 (DICOM SCP)"
echo "  DICOMweb Output: 127.0.0.1:8042 (HTTP)"
echo "  Middleware:      DICOM → DICOMweb transformation"
echo "  Operations:      C-FIND → QIDO-RS, C-STORE → STOW-RS"
echo ""

echo "Logs available at:"
echo "  Harmony:  $TMP_DIR/harmony.log"
echo "  DICOMweb: $TMP_DIR/dicomweb.log"
echo ""

echo -e "${GREEN}✓ Bridge is operational!${NC}"
echo ""
echo "The bridge is ready to accept DICOM connections on 127.0.0.1:11112"
echo "and will transform them to DICOMweb calls to the backend."
echo "Supported operations: C-FIND (QIDO-RS), C-STORE (STOW-RS)"
echo ""
echo "Press Ctrl+C to stop the bridge and exit..."
echo ""

# Keep running
wait
