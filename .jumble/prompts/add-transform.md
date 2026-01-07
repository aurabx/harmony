# Writing JOLT Transforms in Harmony

## Reference Documentation
Full details: `docs/transforms.md`

## Overview

Harmony uses JOLT (JSON to JSON transformation) for data transformations. The transform specification is itself JSON.

## Supported Operations

1. **`shift`** - Move/copy data from input to output
2. **`default`** - Apply default values
3. **`modify-overwrite-beta`** - Modify data with functions (concat, toLower, toUpper, substring, trim, join)
4. **`remove`** - Remove fields from output

## Basic Shift Example

**Input:**
```json
{
    "id": 1,
    "name": "John Smith",
    "account": {
        "id": 1000,
        "type": "Checking"
    }
}
```

**Spec:**
```json
[
    {
        "operation": "shift",
        "spec": {
            "name": "data.name",
            "account": "data.account"
        }
    }
]
```

**Output:**
```json
{
    "data": {
        "name": "John Smith",
        "account": {
            "id": 1000,
            "type": "Checking"
        }
    }
}
```

## Wildcards

### `*` - Match everything
```json
{
    "*": "data.&0"
}
```
Moves all top-level keys under `data`.

### `|` - Match alternatives
```json
{
    "id|name": "data.&(0)"
}
```
Only moves `id` and `name`.

### `&` - Reference matched values
- `&` or `&(0)` or `&(0,0)` = current key
- `&(1)` = parent key
- `&(2)` = grandparent key

## Configuration in Harmony

### 1. Create transform file

Save as `transforms/my-transform.json`:
```json
[
    {
        "operation": "shift",
        "spec": {
            "patient": {
                "name": "resourceType",
                "*": "data.&"
            }
        }
    }
]
```

### 2. Configure middleware

In pipeline TOML:
```toml
[middleware.my_transform]
type = "transform"
[middleware.my_transform.options]
spec_path = "transforms/my-transform.json"
apply_to = "request"  # or "response" or "both"
```

### 3. Add to pipeline
```toml
[pipelines.my_pipeline]
middleware = ["my_transform"]
```

## Common Transform Patterns

### Flatten nested object
```json
{
    "operation": "shift",
    "spec": {
        "user": {
            "profile": {
                "*": "&"
            }
        }
    }
}
```

### Rename fields
```json
{
    "operation": "shift",
    "spec": {
        "old_name": "new_name",
        "nested": {
            "old_field": "nested.new_field"
        }
    }
}
```

### Add defaults
```json
{
    "operation": "default",
    "spec": {
        "status": "active",
        "metadata": {
            "version": "1.0"
        }
    }
}
```

### Remove fields
```json
{
    "operation": "remove",
    "spec": {
        "internal_id": "",
        "debug_info": ""
    }
}
```

### String manipulation
```json
{
    "operation": "modify-overwrite-beta",
    "spec": {
        "name": "=toLower(@(1,name))",
        "full_name": "=concat(@(1,first),' ',@(1,last))"
    }
}
```

## Testing Transforms

### Unit test the transform
```rust
use harmony_jolt::{transform, TransformSpec};

#[test]
fn test_my_transform() {
    let input: Value = serde_json::from_str(r#"{"name": "test"}"#).unwrap();
    let spec: TransformSpec = serde_json::from_str(r#"[...]"#).unwrap();
    
    let output = transform(input, &spec);
    assert_eq!(output["data"]["name"], "test");
}
```

### Test via HTTP
```bash
curl -X POST http://localhost:8080/endpoint \
  -H "Content-Type: application/json" \
  -d '{"name": "test"}'
```

## Debugging Tips

1. **Start simple**: Build transforms incrementally
2. **Test each operation**: Run with single operation first
3. **Check paths**: `&(0)` vs `&(1)` confusion is common
4. **Use echo backend**: Route through echo to see transformed output
