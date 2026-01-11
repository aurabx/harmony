# Transforms

## Overview
JSON to JSON transformation where the "specification" for the transform is itself a JSON document.

Port of Java [Jolt](https://github.com/bazaarvoice/jolt/blob/master/jolt-core/src/main/java/com/bazaarvoice/jolt/Shiftr.java)  library written in Rust.

This is a fork of [Fluvio Jolt](https://github.com/infinyon/fluvio-jolt), created by the Fluvio team. That project was archived on March 2nd 2025.


## Usage Example
Add `harmony-jolt` crate to your `Cargo.toml` file:
```toml
[dependencies]
harmony-jolt = { version = "0.4"}
```

Then, for example, if you want to repack your JSON record, you can do the following:
```rust
use serde_json::{json, Value};
use harmony_jolt::{transform, TransformSpec};

let input: Value = serde_json::from_str(r#"
    {
        "id": 1,
        "name": "John Smith",
        "account": {
            "id": 1000,
            "type": "Checking"
        }
    }
"#).unwrap();

let spec: TransformSpec =
serde_json::from_str(r#"[
    {
      "operation": "shift",
      "spec": {
        "name": "data.name",
        "account": "data.account"
      }
    }
  ]"#).unwrap();

let output = transform(input, &spec);

assert_eq!(output, json!({
    "data" : {
      "name": "John Smith",
      "account": {
        "id": 1000,
        "type": "Checking"
      }
    }
}));
```
## Supported Operations
1. `shift`: copy data from the input tree and put it the output tree
2. `default`: apply default values to the tree
3. `modify-overwrite-beta`: modify data using functions (concat, toLower, toUpper, substring, trim, join)
4. `remove`: remove data from the tree

See `SPEC.md` for more info on specifics of execution order and DSL grammar.

## Specification

Composes a list of operation specifications. Each operation has its own DSL (Domain Specific
Language) in order to facilitate its narrow job.

```
use harmony_jolt::TransformSpec;

let spec: TransformSpec =
serde_json::from_str(r#"[
    {
      "operation": "shift",
      "spec": {
        "name": "data.name",
        "account": "data.account"
      }
    }
  ]"#).unwrap();
```

### <a name="shift"></a>`Shift` operation
Specifies where the data from the input JSON should be placed in the output JSON, or in other
words, how the input JSON/data should be shifted around to make the output JSON/data.

At a base level, a single `shift` operation is a mapping from an input path to an output path,
similar to the `mv` command in Unix, `mv /var/data /var/backup/data`.

The input path is a JSON tree structure, and the output path is flattened "dot notation" path
notation.

For example, given this simple input JSON:
 <pre>
{
    "id": 1,
    "name": "John Smith",
    "account": {
        "id": 1000,
        "type": "Checking"
    }
}
</pre>
A simple spec could be constructed by copying that input, and modifying it to supply an output
path for each piece of data:
<pre>
{
    "id": "data.id",
    "name": "data.name",
    "account": "data.account"
}
</pre>
would produce the following output JSON:
<pre>
{
    "data" : {
        "id": 1,
        "name": "John Smith",
        "account": {
            "id": 1000,
            "type": "Checking"
        }
    }
}
</pre>

### Wildcards
The `shift` specification on the keys level supports wildcards and conditions:  
1. `*` - match everything  
2. `name1|name2|nameN` - match any of the specified names

#### `&` Wildcard
`&` lookup allows referencing the values captured by the `*` or `|`.

`&(x,y)` means go up the path x levels and get the yth match from that level.

0th match is always the entire input they and the rest are the specific things the `*`s matched.

`&` == `&(0)` == `&(0,0)` and `&(x)` == `&(x,0)`

It allows for specs to be more compact. For example, for this input:
 <pre>
{
    "id": 1,
    "name": "John Smith",
    "account": {
        "id": 1000,
        "type": "Checking"
    }
}
</pre>
to get the output:
<pre>
{
    "data" : {
        "id": 1,
        "name": "John Smith",
        "account": {
            "id": 1000,
            "type": "Checking"
        }
    }
}
</pre>
the spec with wildcards would be:
<pre>
{
    "*": "data.&0"
}
</pre>
If you want only `id` and `name` in the output, the spec is:
<pre>
{
    "id|name": "data.&(0)"
}
</pre>


`&` wildcard also allows to dereference any level of the path of given node:
<pre>
{
    "foo": {
        "bar" : {
            "baz": "new_location.&(0).&(1).&(2)" // &(0) = baz, &(1) = bar, &(2) = foo
            }
        }
    }
}
</pre>
for the input:
<pre>
{
    "foo": {
      "bar": {
        "baz": "value"
      }
    }
  }
</pre>
will produce:
<pre>
{
    "new_location": {
      "baz": {
        "bar": {
          "foo": "value"
        }
      }
    }
}
</pre>

#### `$` Wildcard

`$` wildcard allows accessing matched keys from the path and use them on the right hand side.

See tests in `tests/java/resources/shift` for usage examples.

See [java library docs here](https://github.com/bazaarvoice/jolt/blob/master/jolt-core/src/main/java/com/bazaarvoice/jolt/Shiftr.java).

#### `@` Wildcard

`@` wildcard allows accessing values of matched keys from the path and use them on the right hand side.

See tests in `tests/java/resources/shift` for usage examples.

See [java library docs here](https://github.com/bazaarvoice/jolt/blob/master/jolt-core/src/main/java/com/bazaarvoice/jolt/Shiftr.java).

### `Default` operation
Applies default values if the value is not present in the input JSON.

For example, given this simple input JSON:
 <pre>
{
    "phones": {
        "mobile": 01234567,
        "country": "US"
    }
}
</pre>
with the following specification for `default` operation:
<pre>
{
    "phones": {
        "mobile": 0000000,
        "code": "+1"
    }
}
</pre>
the output JSON will be:
<pre>
{
    "phones": {
        "mobile": 01234567,
        "country": "US",
        "code": "+1"
    }
}
</pre>
As you can see, the field `mobile` remains not affected while the `code` has a default '+1' value.

### `Modifier` operations
The `modify-overwrite-beta` operation allows you to modify values using functions. This is useful for string manipulation, data transformation, and combining values.

#### Supported Functions
- **`concat`**: Concatenate multiple strings
- **`toLower`**: Convert string to lowercase
- **`toUpper`**: Convert string to uppercase
- **`substring`**: Extract a substring (start, end)
- **`trim`**: Remove leading/trailing whitespace
- **`join`**: Join array elements with a separator

#### Example: Concatenating Names
For example, given this input JSON:
<pre>
{
    "customer": {
        "firstName": "John",
        "lastName": "Doe"
    }
}
</pre>

You can use `shift` to restructure and `modify-overwrite-beta` with `concat` to combine the names:
<pre>
[
    {
        "operation": "shift",
        "spec": {
            "customer": {
                "firstName": "name[0]",
                "lastName": "name[1]"
            }
        }
    },
    {
        "operation": "modify-overwrite-beta",
        "spec": {
            "name": "=concat(@(1,name[0]),' ',@(1,name[1]))"
        }
    }
]
</pre>

This produces:
<pre>
{
    "name": "John Doe"
}
</pre>

The `@(level,path)` syntax means "go up `level` levels in the tree, then access `path`". In this case, `@(1,name[0])` goes up 1 level from the current position to access the root, then gets `name[0]`.

#### Function Syntax
Functions are specified using the `=functionName(args...)` syntax:
- **Literals**: Single-quoted strings like `' '` or `'_'`
- **Path expressions**: `@(level,path)` to reference values
- **Context references**: `^value` to reference context values
- **Identifiers**: Unquoted values that will be looked up

#### Modifier Variants
- **`modify-overwrite-beta`**: Always writes the result, overwriting existing values
- **`modify-default-beta`**: Only writes if the key doesn't exist
- **`modify-define-beta`**: Only writes if the key doesn't exist or is null

### `Remove` operation
Removes content from the input JSON.
The spec structure matches the input JSON structure. The value of fields is ignored.

For example, given this simple input JSON:
 <pre>
{
    "phones": {
        "mobile": 01234567,
        "country": "US"
    }
}
</pre>
you can remove the `country` by the following specification for `remove` operation:
<pre>
{
    "phones": {
        "country": ""
    }
}
</pre>
the output JSON will be:
<pre>
{
    "phones": {
        "mobile": 01234567
    }
}
</pre>

## Contributing

If you'd like to contribute to the project, please read our [Contributing guide](CONTRIBUTING.md).

## License

This project is licensed under the [Apache license](LICENSE).
