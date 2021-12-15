#!/bin/bash
# packages the output of building to a RPM package

set -eu

cargo build --release --target x86_64-unknown-linux-musl -p qwer-ingest
docker run --rm -v $(pwd):/app -v $(pwd)/artifacts:/rpmbuild qwer-ingest-build-container --entrypoint=/bin/bash
