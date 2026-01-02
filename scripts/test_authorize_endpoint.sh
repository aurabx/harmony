#!/bin/bash
# Test the harmony /admin/authorize endpoint directly
# This bypasses the runbeam CLI and calls harmony's management API

set -e

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo "=================================="
echo "Harmony Authorization Endpoint Test"
echo "=================================="
echo ""

# Check if harmony is running
if ! curl -s http://localhost:9090/admin/info > /dev/null 2>&1; then
    echo -e "${RED}✗${NC} Harmony is not running on localhost:9090"
    echo ""
    echo "Please start harmony first:"
    echo "  cargo run --release -- --config config/config.toml"
    echo ""
    exit 1
fi

echo -e "${GREEN}✓${NC} Harmony is running"
echo ""

# Check for user token
USER_TOKEN_FILE="$HOME/.runbeam/auth.json"
if [ ! -f "$USER_TOKEN_FILE" ]; then
    echo -e "${RED}✗${NC} No user authentication found"
    echo ""
    echo "Please login first:"
    echo "  runbeam login"
    echo ""
    exit 1
fi

# Extract user token and gateway code
USER_TOKEN=$(jq -r '.token' "$USER_TOKEN_FILE" 2>/dev/null)
GATEWAY_CODE=$(jq -r '.id' "$HOME/.runbeam/harmony_instances.json" 2>/dev/null | head -1)

if [ -z "$USER_TOKEN" ] || [ "$USER_TOKEN" = "null" ]; then
    echo -e "${RED}✗${NC} Could not read user token from $USER_TOKEN_FILE"
    exit 1
fi

if [ -z "$GATEWAY_CODE" ] || [ "$GATEWAY_CODE" = "null" ]; then
    echo -e "${YELLOW}⚠${NC}  No harmony instances found"
    echo ""
    echo "Please provide gateway code manually:"
    read -p "Gateway code: " GATEWAY_CODE
    
    if [ -z "$GATEWAY_CODE" ]; then
        echo -e "${RED}✗${NC} Gateway code is required"
        exit 1
    fi
fi

echo -e "${GREEN}✓${NC} Found user token"
echo -e "${GREEN}✓${NC} Gateway code: $GATEWAY_CODE"
echo ""

# Call the /admin/authorize endpoint
echo "Calling /admin/authorize endpoint..."
echo ""

RESPONSE=$(curl -s -w "\n%{http_code}" -X POST http://localhost:9090/admin/authorize \
  -H "Authorization: Bearer $USER_TOKEN" \
  -H "Content-Type: application/json" \
  -d "{\"gateway_id\":\"$GATEWAY_CODE\"}")

HTTP_CODE=$(echo "$RESPONSE" | tail -1)
BODY=$(echo "$RESPONSE" | sed '$d')

if [ "$HTTP_CODE" = "201" ]; then
    echo -e "${GREEN}✓${NC} Authorization successful (HTTP 201)"
    echo ""
    echo "$BODY" | jq '.'
    echo ""
    
    # Check if token was saved
    TOKEN_FILE="./tmp/runbeam/auth.json"
    if [ -f "$TOKEN_FILE" ]; then
        echo -e "${GREEN}✓${NC} Machine token saved to: $TOKEN_FILE"
        echo ""
        
        # Display token info
        echo "Token Details:"
        jq -r '"  Gateway ID:   " + .gateway_id, "  Expires At:   " + .expires_at' "$TOKEN_FILE"
        echo ""
        
        # Check logs for cloud polling start
        echo "Checking logs for cloud polling..."
        if grep -q "Starting cloud config polling" ./tmp/harmony_test.log 2>/dev/null; then
            echo -e "${GREEN}✓${NC} Cloud polling started!"
            echo ""
            tail -5 ./tmp/harmony_test.log | grep -i "cloud\|polling" || true
        else
            echo -e "${YELLOW}⚠${NC}  Cloud polling messages not found in logs"
            echo "  Check: tail -f ./tmp/harmony_test.log | grep -i cloud"
        fi
    else
        echo -e "${RED}✗${NC} Token file not found at: $TOKEN_FILE"
        echo "  Expected location based on storage config"
    fi
else
    echo -e "${RED}✗${NC} Authorization failed (HTTP $HTTP_CODE)"
    echo ""
    echo "Response:"
    echo "$BODY" | jq '.' 2>/dev/null || echo "$BODY"
    echo ""
    
    # Common error cases
    if [ "$HTTP_CODE" = "401" ]; then
        echo "Possible causes:"
        echo "  - User token expired or invalid"
        echo "  - JWT secret mismatch between CLI and harmony"
    elif [ "$HTTP_CODE" = "403" ]; then
        echo "Possible causes:"
        echo "  - Gateway belongs to different team"
        echo "  - User doesn't have permission"
    elif [ "$HTTP_CODE" = "500" ]; then
        echo "Possible causes:"
        echo "  - Cannot connect to Runbeam Cloud API"
        echo "  - Storage backend error"
        echo "  Check harmony logs: tail -50 ./tmp/harmony_test.log"
    fi
fi

echo ""
echo "=================================="
