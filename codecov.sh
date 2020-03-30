#!/bin/bash
for file in ./target/debug/boomerang-*[^\.d]; do
    echo "Running coverage on $file"
    mkdir -p "target/cov/$(basename $file)";
    kcov --exclude-pattern=/.cargo,/usr/lib --verify "target/cov/$(basename $file)" "$file";
done && \
for file in ./target/debug/boomerang_derive-*[^\.d]; do
    echo "Running coverage on $file"
    mkdir -p "target/cov/$(basename $file)";
    kcov --exclude-pattern=/.cargo,/usr/lib --verify "target/cov/$(basename $file)" "$file";
done && \
bash <(curl -s https://codecov.io/bash) -t $CODECOV_TOKEN && \
echo "Uploaded code coverage"