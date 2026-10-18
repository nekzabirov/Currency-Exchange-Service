# Error Codes

Every refusal carries a stable, machine-readable code in the gRPC response metadata under
the key **`x-error-code`**. The gRPC status alone is not enough: "currency not in the
registry" and "rate never arrived" are both `FAILED_PRECONDITION`, yet different people fix
them in different ways. Do not parse the message text. It is written for humans and may
change.

| Code               | gRPC status           | Meaning | Who fixes it |
|--------------------|-----------------------|---------|--------------|
| `UNKNOWN_CURRENCY` | `FAILED_PRECONDITION` | The currency code is not in the registry. | Add it through the seed or a migration. A retry will not help. |
| `RATE_MISSING`     | `FAILED_PRECONDITION` | The currency exists but has never received a rate (checked by `ConvertUnits` only), or its stored rate or scale is unusable. | Run `sync` and check the provider's key and logs. |
| `INVALID_AMOUNT`   | `INVALID_ARGUMENT`    | The amount is NaN or infinite, or the result does not fit in `int64`. | The caller. |
| `INTERNAL`         | `INTERNAL`            | Reserved for unexpected failures. The current code does not emit it. | The service maintainers. |

A **stale** rate is not an error. It is served, and a `rate.stale` warning is logged.

Calls that exceed the 5-second server timeout end with `CANCELLED` and no `x-error-code`.

## Reading the code

Rust (tonic):

```rust
match client.convert_units(request).await {
    Err(status) => {
        let code = status
            .metadata()
            .get("x-error-code")
            .and_then(|v| v.to_str().ok());
        // Some("UNKNOWN_CURRENCY") | Some("RATE_MISSING") | ...
    }
    Ok(response) => { /* ... */ }
}
```

grpcurl prints it as a response trailer with `-v`.

## Sync failures

The `sync` job is not a gRPC endpoint. It reports provider failures in its logs
(`sync.provider_failed`, `sync.provider_unknown`) and exits non-zero. Provider errors have
three kinds:

| Kind      | Example message | Retry helps? |
|-----------|-----------------|--------------|
| Vendor    | `coinbase refused the call: HTTP 400 ...` | no: wrong key, plan or base |
| Transport | `openexchange unreachable: ...` | yes: timeout, DNS, TLS |
| Config    | `openexchange config is missing ...` | no: set the variable |
