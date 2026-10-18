-- Currency registry: the service's only table.
--
-- Two columns that are easy to mix up, and the price of mixing them up is a balance off by orders
-- of magnitude:
--
--   units_scale  how many storage units make one human-readable unit. Today it is 1e9 for EVERY
--                currency: the scale does not depend on the currency. The column is per currency
--                not because the values differ, but so that one day they can be changed with a
--                database row rather than a release of every consuming service.
--   rate         how many units of this currency one base unit (EUR) buys. This is the exchange
--                rate; the sync moves it.
--
-- numeric(40,18): eighteen fractional digits are enough both for bitcoin (around 1e-5 per euro)
-- and for the other side of an exchange. double is not acceptable here: this is money.
CREATE TABLE IF NOT EXISTS currency (
    code            text          PRIMARY KEY,
    kind            text          NOT NULL,
    provider        text          NOT NULL,
    units_scale     bigint        NOT NULL,
    rate            numeric(40,18) NOT NULL DEFAULT 1,
    rate_updated_at timestamptz,
    created_at      timestamptz   NOT NULL DEFAULT now(),
    updated_at      timestamptz   NOT NULL DEFAULT now(),

    CONSTRAINT currency_code_ck  CHECK (code ~ '^[A-Z0-9]{2,12}$'),
    CONSTRAINT currency_kind_ck  CHECK (kind IN ('FIAT', 'CRYPTO')),
    -- A zero rate does not mean "the currency is worthless"; it is an unsynced row on which an
    -- exchange would silently zero the amount. Refuse it on write instead of surprising on convert.
    CONSTRAINT currency_rate_ck  CHECK (rate > 0),
    CONSTRAINT currency_scale_ck CHECK (units_scale > 0)
);

-- The sync walks currencies grouped by adapter.
CREATE INDEX IF NOT EXISTS currency_provider_idx ON currency (provider);
