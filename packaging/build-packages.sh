#!/bin/sh
# Builds .deb and .rpm packages from the release tarballs.
#
#   packaging/build-packages.sh v0.2.0 dist
#
# DIR holds whatchanged-TAG-ARCH-unknown-linux-musl.tar.gz for one or more
# of x86_64 and aarch64; the packages are written next to them. The binary
# is static, so one package per architecture runs on every release of
# every deb- or rpm-based distro. Needs dpkg-deb and rpmbuild.
set -eu

tag=$1
dir=$(cd "$2" && pwd)
version=${tag#v}
summary="Show a timeline of what changed on a Linux machine"
url="https://github.com/majidninja/whatchanged"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for arch in x86_64 aarch64; do
    tarball="$dir/whatchanged-$tag-$arch-unknown-linux-musl.tar.gz"
    [ -f "$tarball" ] || continue
    src="$work/src-$arch"
    mkdir -p "$src"
    tar -xzf "$tarball" -C "$src" --strip-components=1

    # The files as they land on the target machine, shared by both formats.
    tree="$work/tree-$arch"
    install -Dm755 "$src/whatchanged" "$tree/usr/bin/whatchanged"
    install -Dm644 "$src/whatchanged.1" "$tree/usr/share/man/man1/whatchanged.1"
    gzip -9n "$tree/usr/share/man/man1/whatchanged.1"
    install -Dm644 "$src/README.md" "$tree/usr/share/doc/whatchanged/README.md"

    # --- .deb -----------------------------------------------------------
    case $arch in x86_64) debarch=amd64 ;; aarch64) debarch=arm64 ;; esac
    deb="$work/deb-$arch"
    cp -a "$tree" "$deb"
    install -Dm644 "$src/LICENSE" "$deb/usr/share/doc/whatchanged/copyright"
    mkdir "$deb/DEBIAN"
    cat > "$deb/DEBIAN/control" <<CONTROL
Package: whatchanged
Version: $version
Architecture: $debarch
Maintainer: majidninja <networkplus3@gmail.com>
Installed-Size: $(du -sk --exclude=DEBIAN "$deb" | cut -f1)
Section: utils
Priority: optional
Homepage: $url
Description: $summary
 When something breaks and it worked yesterday, whatchanged puts what
 changed since then on one timeline: package installs and upgrades, new
 kernels, boots, edited config files, systemd units, crontabs and shell
 startup files. It only reads what is already on disk.
CONTROL
    dpkg-deb --root-owner-group -Zxz --build "$deb" \
        "$dir/whatchanged_${version}_$debarch.deb" >/dev/null

    # --- .rpm -----------------------------------------------------------
    rpm="$work/rpm-$arch"
    mkdir -p "$rpm/SPECS"
    cp -a "$tree" "$rpm/files"
    install -Dm644 "$src/LICENSE" "$rpm/files/usr/share/licenses/whatchanged/LICENSE"
    cat > "$rpm/SPECS/whatchanged.spec" <<SPEC
Name:           whatchanged
Version:        $version
Release:        1
Summary:        $summary
License:        MIT
URL:            $url
AutoReqProv:    no

%description
When something breaks and it worked yesterday, whatchanged puts what
changed since then on one timeline: package installs and upgrades, new
kernels, boots, edited config files, systemd units, crontabs and shell
startup files. It only reads what is already on disk.

%install
mkdir -p %{buildroot}
cp -a $rpm/files/. %{buildroot}/

%files
%attr(0755, root, root) /usr/bin/whatchanged
%attr(0644, root, root) /usr/share/man/man1/whatchanged.1.gz
%dir %attr(0755, root, root) /usr/share/doc/whatchanged
%doc %attr(0644, root, root) /usr/share/doc/whatchanged/README.md
%dir %attr(0755, root, root) /usr/share/licenses/whatchanged
%license %attr(0644, root, root) /usr/share/licenses/whatchanged/LICENSE
SPEC
    # A prebuilt static binary: nothing to strip, scan or split off, and
    # settings every rpm since RHEL 7 can read.
    rpmbuild -bb --quiet --target "$arch" \
        --define "_topdir $rpm" \
        --define "_rpmdir $rpm/out" \
        --define "_build_id_links none" \
        --define "debug_package %{nil}" \
        --define "__os_install_post %{nil}" \
        --define "__strip /bin/true" \
        --define "_binary_payload w9.gzdio" \
        --define "_binary_filedigest_algorithm 8" \
        "$rpm/SPECS/whatchanged.spec" >/dev/null
    cp "$rpm/out/$arch/whatchanged-$version-1.$arch.rpm" "$dir/"
done

cd "$dir"
ls whatchanged_*.deb whatchanged-*.rpm >/dev/null
for f in whatchanged_*.deb whatchanged-*.rpm; do
    sha256sum "$f" > "$f.sha256"
done
