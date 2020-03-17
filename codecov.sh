#!/bin/bash
shopt -s extglob
set -x
for file in target/debug/@(boomerang|boomerang_derive)-*[^\.d]; do
    mkdir -p "target/cov/$(basename $file)";
    kcov --exclude-pattern=/.cargo,/usr/lib --verify "target/cov/$(basename $file)" "$file";
done && \
bash <(curl -s https://codecov.io/bash) -t $CODECOV_TOKEN && \
set +X && \
echo "Uploaded code coverage"