# Signed public reliability data

This directory is the versioned, repository-backed input to public reliability
cards. There is no upload service, database, API, or telemetry path.

Create a private consent file and publish locally:

```json
{
  "format_version": "1",
  "consent": true,
  "run_id": "run-...",
  "calibration_bundle": "C:/local/path/calibration.bundle.json",
  "public_data_version": "v1",
  "license": "CC-BY-4.0",
  "authorized_by": "local-reviewer"
}
```

```sh
receipts publish --run-dir ./run --consent ./private-consent.json --out ./public-data
```

Only the generated `<version>/<run-id>.json` belongs in a signed pull request.
Never commit the consent file: it may contain private local paths. Every public
record is a fixed allowlist projection signed by the local executor key.

CI performs strict schema decoding, Ed25519 verification, licence checks,
duplicate detection, secret/private-path scanning, and deterministic static
card generation. Model cards and bare gate results never become outcomes.
