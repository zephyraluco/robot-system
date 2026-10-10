#!/bin/bash
set -e

script=$(readlink -f "$0")
route=$(dirname "$script")

tgt_version=$1
tgt_install_prefix=$2
pkg_name=robot-system

if [ "${tgt_version}" == "" ]; then
    tgt_version=$(git describe --tags 2>/dev/null || echo "0.0.0")
fi


if [ "${tgt_install_prefix}" == "" ]; then
    tgt_install_prefix=/opt/robot-system
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

## 2. copy targets to deb ready dir
stage=${route}/../dist/${pkg_name}
install -d ${stage}/opt/robot-system/bin \
           ${stage}/opt/robot-system/migrations \
           ${stage}/etc/robot-system \
           ${stage}/usr/lib/systemd/system

# 二进制文件
install -m 0755 ${route}/../target/release/rsctl              ${stage}/opt/robot-system/bin/rsctl
install -m 0755 ${route}/../target/release/robot-system-daemon ${stage}/opt/robot-system/bin/robot-system-daemon

# 配置、数据库迁移脚本
install -m 0644 ${route}/../etc/robot-system.conf      ${stage}/etc/robot-system/robot-system.conf
install -m 0644 ${route}/../migrations/*.sql                  ${stage}/opt/robot-system/migrations/

# systemd 单元
install -m 0644 ${route}/../etc/systemd/robot-system.target        ${stage}/usr/lib/systemd/system/robot-system.target
install -m 0644 ${route}/../etc/systemd/robot-system-daemon.service ${stage}/usr/lib/systemd/system/robot-system-daemon.service

## 3. make various config files under DEBIAN dir
cd ${route}/../dist/${pkg_name}/DEBIAN
touch control
(cat << EOF
Package: ${pkg_name}
Version: ${tgt_version}
Section: admin
Priority: optional
Depends: libgcc-s1, libc6 (>= 2.39), dpkg, systemd
Architecture: ${arch}
Maintainer: robot-system maintainers
Description: Robot System custom software and runtime manager
 rsctl provides package, deployment and service management; robot-system-daemon
 collects process runs, metrics and error events.
EOF
) > control

touch postinst
(cat << EOF
#!/bin/bash
set -e
case "\$1" in
    configure)
        install -d -m 0755 /var/lib/robot-system \\
                            /var/lib/robot-system/backups \\
                            /var/lib/robot-system/transactions \\
                            /var/log/robot-system \\
                            /run/robot-system
        ln -sf /opt/robot-system/bin/rsctl /usr/bin/rsctl
        if command -v systemctl >/dev/null 2>&1; then
            systemctl daemon-reload || true
            systemctl enable robot-system.target || true
            systemctl enable robot-system-daemon.service || true
            systemctl start robot-system.target || true
            systemctl start robot-system-daemon.service || true
        fi
        # 补全提示：completion 子命令输出的是可直接 eval 的 shell 脚本
        echo ""
        echo "rsctl 命令补全（可选）：把对应的一行加入配置文件后重开终端"
        echo '  bash  ~/.bashrc                    eval "\$(rsctl completions bash)"'
        echo '  zsh   ~/.zshrc                     eval "\$(rsctl completions zsh)"'
        echo '  fish  ~/.config/fish/config.fish   rsctl completions fish | source'
        echo ""
        ;;
esac
exit 0
EOF
) > postinst

touch postrm
(cat << EOF
#!/bin/bash
set -e
case "\$1" in
    remove|purge)
        rm -f /usr/bin/rsctl
        rm -f /etc/systemd/system/multi-user.target.wants/robot-system.target
        rm -f /etc/systemd/system/robot-system.target.wants/robot-system-daemon.service
        if command -v systemctl >/dev/null 2>&1; then
            systemctl disable robot-system-daemon.service || true
            systemctl daemon-reload || true
        fi
        ;;
esac
if [ "\$1" = purge ]; then
    rm -rf /var/lib/robot-system /var/log/robot-system /run/robot-system
fi
exit 0
EOF
) > postrm

touch preinst
(cat << EOF
#!/bin/bash
EOF
) > preinst

touch prerm
(cat << EOF
#!/bin/bash
set -e
case "\$1" in
    remove|deconfigure|upgrade)
        # 只停常驻服务：业务服务属于其它包，绝不触碰
        if command -v systemctl >/dev/null 2>&1; then
            systemctl stop robot-system-daemon.service || true
        fi
        ;;
esac
exit 0
EOF
) > prerm

chmod +x postinst postrm preinst prerm

## 4. start to make .deb package
cd ${route}/..
fakeroot dpkg -b dist/${pkg_name} dist/${pkg_name}_${tgt_version}_${arch}.deb || exit 40
rm -rf dist/${pkg_name}
echo "pack ${pkg_name} into deb finished."
exit 0
