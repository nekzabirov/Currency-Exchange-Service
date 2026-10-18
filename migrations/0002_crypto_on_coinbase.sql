-- Crypto moves from CoinGecko to Coinbase.
--
-- CoinGecko answers keyless calls from many cloud/datacenter IP ranges with an HTTP 403, so every
-- hourly sync failed and these currencies kept stale rates. The seed only inserts missing rows and
-- never rewrites an existing one, so a live registry can change its provider only here.
UPDATE currency SET provider = 'coinbase', updated_at = now() WHERE provider = 'coingecko';
