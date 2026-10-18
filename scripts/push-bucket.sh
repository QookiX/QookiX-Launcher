#!/usr/bin/env bash
# Upload a release directory to the update bucket, then prune stale objects.
#
# Usage: bash scripts/push-bucket.sh <release-dir>
# Required env: QOOKIX_S3_BUCKET
# Optional env: AWS_ENDPOINT_URL, QOOKIX_BUCKET_UPDATE_URL
#
# Uploads are sequential: S3-compatible endpoints throttle concurrent writes
# (SlowDownWrite), especially for multipart uploads of the installers.
set -euo pipefail

release_dir="${1:?usage: push-bucket.sh <release-dir>}"
bucket="${QOOKIX_S3_BUCKET:?QOOKIX_S3_BUCKET is not set}"

export AWS_MAX_ATTEMPTS="${AWS_MAX_ATTEMPTS:-10}"
export AWS_RETRY_MODE="${AWS_RETRY_MODE:-standard}"
aws configure set default.s3.max_concurrent_requests 1

endpoint_args=()
if [ -n "${AWS_ENDPOINT_URL:-}" ]; then
  endpoint_args=(--endpoint-url "${AWS_ENDPOINT_URL}")
fi

upload() {
  local file="$1"
  echo "Uploading ${file}"
  aws "${endpoint_args[@]}" s3 cp "${file}" "s3://${bucket}/" \
    --no-progress --checksum-algorithm CRC32
  sleep 2
}

# Binaries first, manifests last: a manifest must never point at missing assets.
find "${release_dir}" -type f \
  ! -name 'latest.json' ! -name 'release.json' ! -name 'changelog.json' \
  -print0 | sort -z | while IFS= read -r -d '' file; do
  upload "${file}"
done

for file in changelog.json release.json latest.json; do
  if [ -f "${release_dir}/${file}" ]; then
    upload "${release_dir}/${file}"
  fi
done

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
