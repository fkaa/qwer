#!/bin/bash

# change and setup RPM build directories
echo "%_topdir /app/rpmbuild" > ~/.rpmmacros

rpmdev-setuptree
rpmbuild -bb ./build/qwer.spec
