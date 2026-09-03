#!/bin/bash
set -e

script=$(readlink -f "$0")
route=$(dirname "$script")

tgt_version=$1
tgt_install_prefix=$2
pkg_name=robot-system

if [ "${tgt_version}" == "" ]; then
    tgt_version=$(git describe --tag 2>/dev/null || echo "0.0.0")
fi

if [ "${tgt_install_prefix}" == "" ]; then
    tgt_install_prefix=/usr/local
fi
echo "deb install prefix is ${tgt_install_prefix}"

uname_arch=$(uname -m)
if [ x"${uname_arch}" == x"x86_64" ]; then
    arch=amd64
elif [ x"${uname_arch}" == x"aarch64" ]; then
    arch=arm64
else
    echo "not support arch ${uname_arch}" >&2
    exit 2
fi

### 1. make the working dir
if [ -e ${route}/../dist ]; then
    rm -rf ${route}/../dist
fi
mkdir -p ${route}/../dist/${pkg_name}
mkdir -p ${route}/../dist/${pkg_name}/DEBIAN
mkdir -p ${route}/../dist/${pkg_name}/${tgt_install_prefix}/bin
mkdir -p ${route}/../dist/${pkg_name}/etc/robot-system
mkdir -p ${route}/../dist/${pkg_name}/etc/systemd/system

## 2. copy targets to deb ready dir
cp ${route}/../target/release/rsvm ${route}/../dist/${pkg_name}/${tgt_install_prefix}/bin/
cp ${route}/../target/release/rsctl ${route}/../dist/${pkg_name}/${tgt_install_prefix}/bin/
cp ${route}/../etc/robot-system.conf ${route}/../dist/${pkg_name}/etc/robot-system/
cp ${route}/../etc/robot-system.target ${route}/../dist/${pkg_name}/etc/systemd/system/

## 3. make various config files under DEBIAN dir
cd ${route}/../dist/${pkg_name}/DEBIAN
touch control
(cat << EOF
Package: ${pkg_name}
Version: ${tgt_version}
Section: utils
Priority: optional
Architecture: ${arch}
Maintainer: zeal
Description: Command-line tools for managing and controlling the robot system.
EOF
) > control

## 4. start to make .deb package
cd ${route}/..
fakeroot dpkg -b dist/${pkg_name} dist/${pkg_name}_${tgt_version}_${arch}.deb || exit 40
rm -rf dist/${pkg_name}
echo "pack ${pkg_name} into deb finished."
exit 0
