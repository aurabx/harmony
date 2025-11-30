#!/bin/bash
# Integration test runner for dicom_to_dicomweb middleware
# This script runs the full end-to-end tests with DCMTK tools

set -e

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${GREEN}=== DICOM to DICOMweb Integration Test Runner ===${NC}\n"

# Check prerequisites
echo "Checking prerequisites..."

# Check Python3
if ! command -v python3 &> /dev/null; then
    echo -e "${RED}ERROR: python3 is not installed${NC}"
    exit 1
fi
echo -e "${GREEN}✓${NC} Python3 found: $(python3 --version)"

# Check DCMTK tools
MISSING_TOOLS=()
for tool in echoscu storescu findscu getscu movescu; do
    if ! command -v $tool &> /dev/null; then
        MISSING_TOOLS+=($tool)
    fi
done

if [ ${#MISSING_TOOLS[@]} -ne 0 ]; then
    echo -e "${RED}ERROR: DCMTK tools missing: ${MISSING_TOOLS[*]}${NC}"
    echo "Install with: brew install dcmtk (macOS) or sudo apt-get install dcmtk (Ubuntu)"
    exit 1
fi
echo -e "${GREEN}✓${NC} All DCMTK tools found"

# Check sample DICOM files
if [ ! -f "samples/dicom/study_1/series_1/CT.1.1.dcm" ]; then
    echo -e "${RED}ERROR: Sample DICOM files not found${NC}"
    echo "Expected: samples/dicom/study_1/series_1/CT.1.1.dcm"
    exit 1
fi
echo -e "${GREEN}✓${NC} Sample DICOM files found"

echo -e "\n${GREEN}All prerequisites met!${NC}\n"

# Run tests
echo "Running integration tests..."
echo -e "${YELLOW}Note: Tests are marked with #[ignore] - using --ignored flag${NC}\n"

# Check if a specific test was requested
if [ $# -eq 0 ]; then
    # Run all tests
    echo "Running all integration tests..."
    cargo test --test dicom_to_dicomweb_integration -- --ignored --nocapture
else
    # Run specific test
    TEST_NAME=$1
    echo "Running test: $TEST_NAME"
    cargo test --test dicom_to_dicomweb_integration $TEST_NAME -- --ignored --nocapture
fi

EXIT_CODE=$?

if [ $EXIT_CODE -eq 0 ]; then
    echo -e "\n${GREEN}✓ All tests passed!${NC}"
else
    echo -e "\n${RED}✗ Some tests failed${NC}"
    echo "Check logs at: ./tmp/dicom_to_dicomweb_integration.log"
fi

exit $EXIT_CODE
