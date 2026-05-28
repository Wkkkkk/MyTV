#!/bin/sh
set -e

cp scripts/pre-push.sh .git/hooks/pre-push
chmod +x .git/hooks/pre-push
echo "Installed .git/hooks/pre-push"
