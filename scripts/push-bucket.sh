#!/usr/bin/env bash
# QOOKIX_S3_BUCKET, QOOKIX_S3_PREFIX and QOOKIX_BUCKET_UPDATE_URL identify the
# managed update directory. AWS_ENDPOINT_URL is optional.
set -euo pipefail
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec node "${script_dir}/push-bucket.mjs" "$@"
