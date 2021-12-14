#!/bin/bash
# packages the output of building to a RPM package

cargo build --release --target x86_64-unknown-linux-musl -p qwer &&
npm run build --prefix ./qw-site/frontend &&
docker run --rm -it -v $(pwd):/app -v $(pwd)/artifacts:/rpmbuild qwer-build-container --entrypoint=/bin/bash
