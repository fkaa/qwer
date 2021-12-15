#!/bin/bash
# packages the output of building to a RPM package

set -eu

npm run build --prefix ./qw-site/frontend
cargo build --release --target x86_64-unknown-linux-musl -p qwer
docker run --rm -v $(pwd):/app -v $(pwd)/artifacts:/rpmbuild qwer-build-container --entrypoint=/bin/bash
