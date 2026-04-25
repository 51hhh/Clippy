#!/bin/sh
strToken=""
ver="latest"
domain="openp2p.cn"
maxLogSize=1048576
usage() {
    cat << EOT

Usage :  ${0} [OPTION] ...
  install openp2p

Options:
  --token        token string
  --ver          version string (default: latest)
EOT
}

while [ $# -gt 0 ]; do
    case "$1" in
        --token )
            shift
            strToken="$1"
            shift
        ;;
        --ver )
            shift
            ver="$1"
            shift
        ;;
        --domain )
            shift
            domain="$1"
            shift
        ;;
        --help )
            usage
            exit 0
        ;;
        * )
            usage
            exit 1
        ;;
    esac
done

if [ -z "$strToken" ]; then
    echo "token empty"
    exit 1
fi

if [ "$(id -u)" -ne 0 ]; then
    echo "Need root privileges"
    exit 1
fi

sysType=$(uname -s)
echo "system: ${sysType}"
archType=$(uname -m)
echo "arch: ${archType}"
if [ "$sysType" = "Darwin" ]; then
    sysType="darwin-amd64"
    if [ "$archType" = "arm64" ]; then
        sysType="darwin-arm64"
    fi
elif [ "$sysType" = "Linux" ]; then
    sysType="linux-amd64"
    case "$archType" in
        aarch64)
            sysType="linux-arm64"
        ;;
        arm*)
            sysType="linux-arm"
        ;;
        i*86)
            sysType="linux-386"
        ;;
        s390x)
            sysType="linux-s390x"
        ;;
        mips)
            num=$(printf '\x01\x02\x03\x04' | hexdump | awk '{print $2}')
            echo "num is $num"
            if [ "$num" = "0102" ]; then
                sysType="linux-mips"
                echo "endian: big"
            else
                sysType="linux-mipsle"
                echo "endian: little"
            fi
        ;;
    esac
fi

if grep -qi "openwrt" /etc/os-release || grep -qi "openwrt" /etc/openwrt_release || which opkg >/dev/null 2>&1; then
    echo "✅ is OpenWrt"
    maxLogSize=102400
    if [ ! -c /dev/net/tun ]; then
        echo "❌ /dev/net/tun not exist, installing kmod-tun..."
        opkg update
        opkg install kmod-tun
    else
        echo "✔ /dev/net/tun exist"
    fi
fi
url="https://${domain}/download/v1/${ver}/openp2p-${ver}.${sysType}.tar.gz"
echo "download $url start"
cd /tmp
rm -rf /tmp/openp2p/
if command -v curl >/dev/null; then
    curl -k --max-time 30 -o  openp2p.tar.gz "$url"
else
    wget --timeout=30 --no-check-certificate -O openp2p.tar.gz "$url"
fi
if [ $? -ne 0 ]; then
    echo "download error $?, retry..."
    url="https://console.openpxp.com/download/v1/${ver}/openp2p-${ver}.${sysType}.tar.gz"
    echo "download $url start"

    if command -v curl >/dev/null; then
        curl -k -o openp2p.tar.gz "$url"
    else
        wget --no-check-certificate -O openp2p.tar.gz "$url"
    fi
    if [ $? -ne 0 ]; then
    echo "download error $?"
    exit 9
    fi
fi
echo "download ok"
tar -xzvf openp2p.tar.gz
chmod +x openp2p
echo "install start"
echo "max log size is ${maxLogSize}"
./openp2p install -token $strToken -maxlogsize $maxLogSize
if [ $? -ne 0 ]; then
    echo "install error $?"
    exit 9
fi
rm openp2p.tar.gz openp2p
echo "install ok"
