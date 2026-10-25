# ADR 0007: Secrets in the OS keyring

Status: accepted (architect, 2026-10-24; PLAN §4 D20, §6.4)

## Context

Berth stores registry passwords and tokens, the Coolify API token (through the plugin host) and TLS key passphrases for remote engines. None may sit in SQLite, logs, fixtures or plugin configs. Tests must not touch a real keychain. `keyring` 4.2 (latest on 2026-10-24) replaced its backend model with separate store crates; `keyring` 3.6.3 with `apple-native` is proven by Ferry in a Developer-ID-signed, hardened-runtime app on this Mac.

## Decision

- `trait SecretStore` in `berth-store` (`get`, `set`, `delete`, values are `secrecy::SecretString`).
- `KeyringStore` on `keyring` 3.6 with `apple-native`; service `dev.berth`; keys `registry/<id>/password`, `plugin/<plugin_id>/<key>`, `engine/<id>/tls-key-passphrase`.
- `InMemorySecretStore` for tests and for `BERTH_KEYRING=memory`. If the keychain fails, Berth falls back to memory and shows a sticky notice.
- No `keychain-access-groups` entitlement (Ferry ADR 0004 addendum).

## Consequences

- No plaintext credential on disk in Berth's data directory; `scripts/secret-scan.sh` guards the repository.
- A re-signed build may make macOS ask once for keychain access; a stable signing identity keeps "Always Allow" valid (Ferry Mac session).
- Moving to keyring 4 is a later ADR.
