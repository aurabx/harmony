#!/bin/bash
# Test script for cloud polling integration
# This verifies that the changes work correctly

set -e

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo "=================================="
echo "Cloud Polling Integration Tests"
echo "=================================="
echo ""

# Find the token file
TOKEN_FILE="./tmp/runbeam/auth.json"
CONFIG_FILE="./config/config.toml"

echo "1. Checking for authorized token..."
if [ -f "$TOKEN_FILE" ]; then
    echo -e "${GREEN}✓${NC} Token file exists: $TOKEN_FILE"
    
    # Display token info
    GATEWAY_ID=$(jq -r '.gateway_id' "$TOKEN_FILE" 2>/dev/null || echo "unknown")
    EXPIRES_AT=$(jq -r '.expires_at' "$TOKEN_FILE" 2>/dev/null || echo "unknown")
    
    echo "  Gateway ID:   $GATEWAY_ID"
    echo "  Expires At:   $EXPIRES_AT"
    
    # Check if token is expired
    if command -v date &> /dev/null; then
        EXPIRY_EPOCH=$(date -j -f "%Y-%m-%dT%H:%M:%S" "$(echo $EXPIRES_AT | cut -d'+' -f1 | cut -d'Z' -f1)" "+%s" 2>/dev/null || echo "0")
        NOW_EPOCH=$(date +%s)
        
        if [ "$EXPIRY_EPOCH" -gt "$NOW_EPOCH" ]; then
            DAYS_LEFT=$(( ($EXPIRY_EPOCH - $NOW_EPOCH) / 86400 ))
            echo -e "  ${GREEN}Token is valid${NC} (expires in $DAYS_LEFT days)"
        else
            echo -e "  ${RED}Token is EXPIRED${NC}"
            echo -e "  ${YELLOW}Re-authorize via: curl -X POST http://localhost:9090/admin/authorize${NC}"
        fi
    fi
    echo ""
else
    echo -e "${YELLOW}⚠${NC}  No token file found at $TOKEN_FILE"
    echo -e "  ${YELLOW}Gateway needs authorization via /admin/authorize endpoint${NC}"
    echo ""
fi

echo "2. Checking configuration..."
if [ -f "$CONFIG_FILE" ]; then
    echo -e "${GREEN}✓${NC} Config file exists: $CONFIG_FILE"
    
    # Check management section
    if grep -q "^\[management\]" "$CONFIG_FILE"; then
        echo -e "${GREEN}✓${NC} Management section configured"
        
        # Check if management is enabled
        if grep -A 5 "^\[management\]" "$CONFIG_FILE" | grep -q "enabled = true"; then
            echo -e "${GREEN}✓${NC} Management API is enabled"
        else
            echo -e "${RED}✗${NC} Management API is NOT enabled"
        fi
        
        # Check polling interval
        POLL_INTERVAL=$(grep "poll_interval_secs" "$CONFIG_FILE" || echo "  Using default: 30 seconds")
        echo "  $POLL_INTERVAL"
    else
        echo -e "${YELLOW}⚠${NC}  Management section not found in config"
    fi
    echo ""
else
    echo -e "${RED}✗${NC} Config file not found: $CONFIG_FILE"
    echo ""
fi

echo "3. Checking /admin/reload endpoint removal..."
if curl -s -f http://localhost:9090/admin/reload > /dev/null 2>&1; then
    echo -e "${RED}✗${NC} /admin/reload still responds (should be removed)"
else
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" http://localhost:9090/admin/reload 2>/dev/null || echo "000")
    if [ "$HTTP_CODE" = "404" ]; then
        echo -e "${GREEN}✓${NC} /admin/reload returns 404 (correctly removed)"
    elif [ "$HTTP_CODE" = "000" ]; then
        echo -e "${YELLOW}⚠${NC}  Cannot connect to management API (is harmony running?)"
    else
        echo -e "${YELLOW}⚠${NC}  /admin/reload returned HTTP $HTTP_CODE (expected 404)"
    fi
fi
echo ""

echo "4. Checking management API endpoints..."
ENDPOINTS=("info" "pipelines" "routes" "config/status")

for endpoint in "${ENDPOINTS[@]}"; do
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:9090/admin/$endpoint" 2>/dev/null || echo "000")
    
    if [ "$HTTP_CODE" = "200" ]; then
        echo -e "${GREEN}✓${NC} /admin/$endpoint is accessible (HTTP 200)"
    elif [ "$HTTP_CODE" = "000" ]; then
        echo -e "${YELLOW}⚠${NC}  Cannot connect to /admin/$endpoint (is harmony running?)"
    else
        echo -e "${RED}✗${NC} /admin/$endpoint returned HTTP $HTTP_CODE (expected 200)"
    fi
done
echo ""

echo "5. Test Summary"
echo "=================================="

if [ -f "$TOKEN_FILE" ]; then
    echo -e "${GREEN}✓${NC} Gateway is authorized"
    echo ""
    echo "Expected behavior:"
    echo "  • Cloud polling should have started automatically"
    echo "  • Check logs for: '🌥️  Found valid stored token' message"
    echo "  • Polling will check for config changes every 30 seconds (default)"
    echo ""
    echo "To verify polling is active, check the logs:"
    echo "  tail -f ./tmp/harmony_test.log | grep -i 'cloud polling'"
else
    echo -e "${YELLOW}⚠${NC}  Gateway needs authorization"
    echo ""
    echo "To authorize and start cloud polling:"
    echo "  1. Get a JWT token from Runbeam Cloud"
    echo "  2. Call: curl -X POST http://localhost:9090/admin/authorize \\"
    echo "              -H 'Authorization: Bearer YOUR_JWT_TOKEN' \\"
    echo "              -H 'Content-Type: application/json' \\"
    echo "              -d '{\"gateway_id\":\"your-gateway-id\"}'"
    echo ""
    echo "After authorization, cloud polling will start automatically!"
fi

echo ""
echo "=================================="
