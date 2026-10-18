#!/usr/bin/env bash
# Upload a release directory to the update bucket, then prune stale objects.
#
# Usage: bash scripts/push-bucket.sh <release-dir>
# Required env: QOOKIX_S3_BUCKET
# Optional env: AWS_ENDPOINT_URL, QOOKIX_BUCKET_UPDATE_URL
#
# Uploads are sequential and each file goes up as a single PUT, so the endpoint
# sees one write per file instead of many multipart `UploadPart` calls — those
# are what trip its write-rate limit (SlowDownWrite) even for ~10 MB installers.
set -euo pipefail

release_dir="${1:?usage: push-bucket.sh <release-dir>}"
bucket="${QOOKIX_S3_BUCKET:?QOOKIX_S3_BUCKET is not set}"

export AWS_MAX_ATTEMPTS="${AWS_MAX_ATTEMPTS:-10}"
export AWS_RETRY_MODE="${AWS_RETRY_MODE:-standard}"

endpoint_args=()
if [ -n "${AWS_ENDPOINT_URL:-}" ]; then
  endpoint_args=(--endpoint-url "${AWS_ENDPOINT_URL}")
fi

upload() {
  local file="$1"
  local key
  key="$(basename "${file}")"
  echo "Uploading ${key}"
  aws "${endpoint_args[@]}" s3api put-object \
    --bucket "${bucket}" \
    --key "${key}" \
    --body "${file}" \
    --checksum-algorithm CRC32 \
    --no-cli-pager >/dev/null
  sleep 2
}

# Binaries first, manifests last: a manifest must never point at missing assets.
# Files listed by list-external-assets.mjs (large AppImages) are served straight
# from GitHub Releases and skipped here, because the endpoint throttles them.
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
skip_list="$(mktemp)"
node "${script_dir}/list-external-assets.mjs" "${release_dir}" | sort -u > "${skip_list}"

find "${release_dir}" -type f \
  ! -name 'latest.json' ! -name 'release.json' ! -name 'changelog.json' \
  -print0 | sort -z | while IFS= read -r -d '' file; do
  if grep -Fxq "$(basename "${file}")" "${skip_list}"; then
    echo "Skipping $(basename "${file}") (served from GitHub Releases)"
    continue
  fi
  upload "${file}"
done

for file in changelog.json release.json latest.json; do
  if [ -f "${release_dir}/${file}" ]; then
    upload "${release_dir}/${file}"
  fi
done

rm -f "${skip_list}"

# Remove objects that are not part of this release (latest.json is overwritten).
aws "${endpoint_args[@]}" s3 ls "s3://${bucket}" --recursive \
  | awk '{print $4}' | sort -u > /tmp/remote-keys.txt
find "${release_dir}" -type f -printf '%f\n' | sort -u > /tmp/local-names.txt
comm -23 /tmp/remote-keys.txt /tmp/local-names.txt > /tmp/stale-keys.txt
if [ -s /tmp/stale-keys.txt ]; then
  echo "Removing stale objects:"
  cat /tmp/stale-keys.txt
  while IFS= read -r key; do
    if [ -n "${key}" ]; then
      aws "${endpoint_args[@]}" s3 rm "s3://${bucket}/${key}"
    fi
  done < /tmp/stale-keys.txt
fi

echo "Uploaded update artifacts to ${QOOKIX_BUCKET_UPDATE_URL:-s3://${bucket}}"
