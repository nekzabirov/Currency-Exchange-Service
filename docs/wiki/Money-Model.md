# Money Model

## Integer units

Balances are stored as `int64` **units**. Each currency has a `units_scale`: the number of
units in one human-readable unit. Today every currency uses `1e9` ("nano"):

| Human-readable | Units |
|----------------|-------|
| `1 EUR`        | `1000000000` |
| `10.50 UAH`    | `10500000000` |
| `0.00000001 BTC` (1 satoshi) | `10` |

The scale does **not** depend on the currency. It is a column per currency
(`currency.units_scale`) so that it could one day change with a database row rather than a
release of every consumer. The formulas already account for different scales.

With a scale of `1e9`, `int64` holds amounts up to about `9.2e9` human-readable units.
Anything beyond that is refused with `INVALID_AMOUNT` and never wraps around.

## Rates

`currency.rate` is **how many units of this currency one unit of the base currency buys**.
The base is `EUR` by default (`BASE_CURRENCY` in `sync`), and its own rate is always `1`.
Rates are `numeric(40,18)` in the database and `rust_decimal::Decimal` in memory. Eighteen
fractional digits cover bitcoin's roughly `1e-5` per euro with room to spare.

A rate of zero is rejected by a `CHECK` constraint. It would mean "unsynced", not "worthless",
and an exchange would silently zero the amount.

## Arithmetic

All arithmetic runs in `Decimal`, never in `f64`. The contract carries amounts as `double`,
and `Decimal::from_f64_retain` takes exactly the decimal digits the double held. From there
everything is exact. A naive `0.1 * 1e9` in floating point gives `100000000.00000001`.
ratekeeper gives `100000000`.

### Representation (`ConvertToUnits` / `ConvertFromUnits`)

```text
units  = round_half_up(amount × units_scale)
amount = units / units_scale
```

No rate is involved. These calls work before the first sync.

### Exchange (`ConvertUnits`)

A cross rate through the base currency, corrected for the scales:

```text
                 units × rate_to × scale_to
units_to = round( ────────────────────────── )
                   rate_from × scale_from
```

The ratio `(rate_to × scale_to) / (rate_from × scale_from)` is computed **first**. `Decimal`
holds 28 significant digits. A balance near `1e18` units multiplied by a rate and a scale
before dividing would overflow them on a perfectly legal amount.

The same currency on both sides is an identity and skips the arithmetic, so rounding can
never take a unit away.

### Rounding

Half-up, away from zero (`MidpointAwayFromZero`), the same as Java's `RoundingMode.HALF_UP`:
`0.25` at scale `2` gives `1`, and `-0.25` gives `-1`. Negative amounts are allowed, because
the sign carries the direction of the movement (for example, a reversal).

### The cost of `double` in the contract

A midpoint below the ninth decimal place cannot be represented as a double. The nearest
double to `1.5e-9` lies slightly below the midpoint and yields `1`, while the nearest to
`2.5e-9` lies slightly above and yields `3`. This is a fraction of a nano that no currency
has, so it never affects money. If exactness down to the last unit is ever needed for
arbitrary inputs, the fix is a string field in the proto, not different rounding.

## Stale and missing rates

| State | Detected by | Behaviour |
|-------|-------------|-----------|
| Fresh (updated within 24 h) | `rate_updated_at` | served |
| Stale (older than 24 h) | `rate_updated_at` | **served**, with a `rate.stale` `WARN` log that carries `ageHours` |
| Missing (never synced) | `rate_updated_at IS NULL` | `ConvertUnits` refuses with `RATE_MISSING` |

See [Error Codes](Error-Codes).
