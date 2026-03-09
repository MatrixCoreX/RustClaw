#!/bin/bash

# System Security Audit Script
# This script performs basic security checks on a Linux system.

set -e

LOG_FILE="security_audit_$(date +%Y%m%d_%H%M%S).log"
echo "=== System Security Audit Report ===" | tee "$LOG_FILE"
echo "Generated on: $(date)" | tee -a "$LOG_FILE"
echo "Hostname: $(hostname)" | tee -a "$LOG_FILE"
echo "Kernel: $(uname -r)" | tee -a "$LOG_FILE"
echo "" | tee -a "$LOG_FILE"

# 1. Check rootkit detection (if rkhunter installed)
echo "[1/7] Checking for rootkits (rkhunter)..." | tee -a "$LOG_FILE"
if command -v rkhunter >/dev/null 2>&1; then
  rkhunter --check --sk --nocolors 2>/dev/null | grep -E "Warning|Suspect|Untrusted" | tee -a "$LOG_FILE" || echo "  OK: No warnings found." | tee -a "$LOG_FILE"
else
  echo "  SKIP: rkhunter not installed." | tee -a "$LOG_FILE"
fi

echo "" | tee -a "$LOG_FILE"

# 2. Check listening network services
echo "[2/7] Listing listening TCP/UDP ports..." | tee -a "$LOG_FILE"
ss -tuln 2>/dev/null | tee -a "$LOG_FILE"

echo "" | tee -a "$LOG_FILE"

# 3. Check sudoers syntax
echo "[3/7] Validating /etc/sudoers syntax..." | tee -a "$LOG_FILE"
if sudo visudo -c 2>&1 | tee -a "$LOG_FILE" | grep -q "correct"; then
  echo "  OK: /etc/sudoers syntax is valid." | tee -a "$LOG_FILE"
else
  echo "  ERROR: /etc/sudoers syntax error!" | tee -a "$LOG_FILE"
fi

echo "" | tee -a "$LOG_FILE"

# 4. Check for world-writable files in critical paths
echo "[4/7] Searching world-writable files in /etc, /bin, /sbin, /usr/bin, /usr/sbin..." | tee -a "$LOG_FILE"
find /etc /bin /sbin /usr/bin /usr/sbin -type f -perm -002 2>/dev/null | head -20 | tee -a "$LOG_FILE"
if [ $(find /etc /bin /sbin /usr/bin /usr/sbin -type f -perm -002 2>/dev/null | wc -l) -eq 0 ]; then
  echo "  OK: No world-writable files found." | tee -a "$LOG_FILE"
else
  echo "  WARNING: Above are up to 20 world-writable files — review manually." | tee -a "$LOG_FILE"
fi

echo "" | tee -a "$LOG_FILE"

# 5. Check password aging policy
echo "[5/7] Checking password aging policy (/etc/login.defs)..." | tee -a "$LOG_FILE"
grep -E "^(PASS_MAX_DAYS|PASS_MIN_DAYS|PASS_WARN_AGE)" /etc/login.defs 2>/dev/null | tee -a "$LOG_FILE"

echo "" | tee -a "$LOG_FILE"

# 6. Check for unattended-upgrades (Debian/Ubuntu) or yum-cron (RHEL/CentOS)
echo "[6/7] Checking automatic security updates..." | tee -a "$LOG_FILE"
if command -v unattended-upgrade >/dev/null 2>&1; then
  echo "  INFO: unattended-upgrades detected." | tee -a "$LOG_FILE"
  systemctl is-active --quiet unattended-upgrades && echo "  OK: unattended-upgrades is active." || echo "  WARN: unattended-upgrades is inactive." | tee -a "$LOG_FILE"
elif command -v yum-cron >/dev/null 2>&1; then
  echo "  INFO: yum-cron detected." | tee -a "$LOG_FILE"
  systemctl is-active --quiet yum-cron && echo "  OK: yum-cron is active." || echo "  WARN: yum-cron is inactive." | tee -a "$LOG_FILE"
else
  echo "  SKIP: No known auto-update service found." | tee -a "$LOG_FILE"
fi

echo "" | tee -a "$LOG_FILE"

# 7. List users with UID 0 (root-equivalent)
echo "[7/7] Checking for UID 0 accounts (beyond root)..." | tee -a "$LOG_FILE"
awk -F: '$3 == 0 && $1 != "root" {print}' /etc/passwd 2>/dev/null | tee -a "$LOG_FILE"
if [ $(awk -F: '$3 == 0 && $1 != "root" {print}' /etc/passwd 2>/dev/null | wc -l) -eq 0 ]; then
  echo "  OK: Only 'root' has UID 0." | tee -a "$LOG_FILE"
else
  echo "  CRITICAL: Non-root UID 0 accounts detected — investigate immediately!" | tee -a "$LOG_FILE"
fi

echo "" | tee -a "$LOG_FILE"
echo "=== Audit completed. Log saved to: $LOG_FILE ===" | tee -a "$LOG_FILE"
