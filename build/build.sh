#!/bin/bash

# change and setup RPM build directories
echo "%_topdir /app/rpmbuild" > ~/.rpmmacros

rpmdev-setuptree

# cp ./target/x86_64-unknown-linux-musl/release/qwer build/qwer
# tar our repository, which is used by the qwer.spec later
# tar --exclude={target,artifacts,.git,node_modules} -vpczf /rpmbuild/SOURCES/qwer.tar.gz --transform "s,^,qwer-$CRATE_VERSION/," . ./target/x86_64-unknown-linux-musl/release/qwer

# build the RPM
rpmbuild -bb ./build/qwer.spec
