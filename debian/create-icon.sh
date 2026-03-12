#!/bin/bash

echo "创建简单的PNG图标..."

convert -size 256x256 xc:none \
    -fill "#4A90E2" \
    -draw "roundrectangle 20,20 236,236 20,20" \
    -fill white \
    -pointsize 80 \
    -gravity center \
    -annotate +0-30 "FTP" \
    -pointsize 40 \
    -annotate +0+40 "Server" \
    /home/GGFWZX/Desktop/wftpg/ui/wftpg.png 2>/dev/null

if [ $? -eq 0 ]; then
    echo "图标创建成功: ui/wftpg.png"
else
    echo "ImageMagick未安装，创建SVG图标..."
    cat > /home/GGFWZX/Desktop/wftpg/ui/wftpg.svg << 'EOF'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <defs>
    <linearGradient id="grad" x1="0%" y1="0%" x2="100%" y2="100%">
      <stop offset="0%" style="stop-color:#4A90E2;stop-opacity:1" />
      <stop offset="100%" style="stop-color:#357ABD;stop-opacity:1" />
    </linearGradient>
  </defs>
  <rect x="20" y="20" width="216" height="216" rx="20" ry="20" fill="url(#grad)"/>
  <text x="128" y="110" font-family="Arial, sans-serif" font-size="60" font-weight="bold" fill="white" text-anchor="middle">FTP</text>
  <text x="128" y="160" font-family="Arial, sans-serif" font-size="30" fill="white" text-anchor="middle">Server</text>
</svg>
EOF
    echo "SVG图标创建成功: ui/wftpg.svg"
fi

echo "完成!"
