#!/bin/bash

# change and setup RPM build directories
echo "%_topdir /rpmbuild" > ~/.rpmmacros

export CRATE_VERSION=$(rg '^version = "([0-9].+)"' ./qw-ingest/Cargo.toml --replace '$1')
echo "Building RPM package for version $CRATE_VERSION"

rpmdev-setuptree

cp ./target/x86_64-unknown-linux-musl/release/qwer-ingest build/qwer-ingest
# tar our repository, which is used by the qwer.spec later
tar --exclude={target,artifacts,.git,node_modules} -vpczf /rpmbuild/SOURCES/qwer-ingest.tar.gz --transform "s,^,qwer-ingest-$CRATE_VERSION/," . ./target/x86_64-unknown-linux-musl/release/qwer-ingest

# build the RPM
rpmbuild -bb ./build/qwer-ingest.spec

INGEST_RTMP_ADDR=0.0.0.0:1935

INGEST_WEB_ADDR=localhost:8080
INGEST_RPC_ADDR=localhost:8081

SCUFFED_RPC_ADDR=http://localhost:9081

RUST_LOG=debug,h2=warn,tower=warn
