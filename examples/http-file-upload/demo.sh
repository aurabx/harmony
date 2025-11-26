#!/bin/bash
set -e

# Change to project root
cd "$(dirname "$0")/../.."

# Create a test file
echo "This is a test file content for upload demo." > test_upload.txt
echo "Created test_upload.txt"

# Ensure clean state
rm -rf examples/http-file-upload/tmp
mkdir -p examples/http-file-upload/tmp/uploads

# Start Harmony in background
echo "Starting Harmony Proxy..."
cargo run -- --config examples/http-file-upload/config.toml > examples/http-file-upload/server.log 2>&1 &
SERVER_PID=$!

# Wait for server to start (loop up to 60s)
echo "Waiting for server to start..."
MAX_RETRIES=30
for i in $(seq 1 $MAX_RETRIES); do
    if curl -s http://127.0.0.1:8080/health > /dev/null; then
        echo "Server is up!"
        break
    fi
    # If not health endpoint, check if port is open by trying connect
    if nc -z 127.0.0.1 8080 2>/dev/null; then
         echo "Server port is open!"
         break
    fi
    
    if [ "$i" -eq "$MAX_RETRIES" ]; then
        echo "Timeout waiting for server to start"
        cat examples/http-file-upload/server.log
        exit 1
    fi
    echo -n "."
    sleep 2
done
echo ""

# Function to cleanup
cleanup() {
    echo "Stopping server..."
    kill $SERVER_PID
    rm test_upload.txt
}
trap cleanup EXIT

# Upload the file
echo "Uploading file..."
RESPONSE=$(curl -s -X POST http://127.0.0.1:8080/upload --data-binary @test_upload.txt)

echo "Response: $RESPONSE"

# Check if response contains location
LOCATION=$(echo $RESPONSE | grep -o '"location":"[^"]*"' | cut -d'"' -f4)

if [ -z "$LOCATION" ]; then
    echo "❌ Upload failed, no location returned"
    cat examples/http-file-upload/server.log
    exit 1
fi

echo "✅ File uploaded to: $LOCATION"

# Verify file exists on disk
# The location is typically full path or relative to root. 
# Our root is ./tmp/uploads, and write_pattern is files/{uuid}.bin
# The location returned by storage backend is {root}/{path}
# So if root is ./tmp/uploads, location is ./tmp/uploads/files/UUID.bin

# Check if file exists
if ls examples/http-file-upload/tmp/uploads/files/*.bin 1> /dev/null 2>&1; then
    UPLOADED_FILE=$(ls examples/http-file-upload/tmp/uploads/files/*.bin | head -n 1)
    echo "✅ Verified file exists on disk: $UPLOADED_FILE"
    
    # Verify content
    CONTENT=$(cat "$UPLOADED_FILE")
    if [ "$CONTENT" == "This is a test file content for upload demo." ]; then
         echo "✅ Content matches!"
    else
         echo "❌ Content mismatch!"
         echo "Expected: This is a test file content for upload demo."
         echo "Got: $CONTENT"
         exit 1
    fi
else
    echo "❌ File not found on disk"
    ls -R examples/http-file-upload/tmp
    exit 1
fi
