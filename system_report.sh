#!/bin/bash

# 系统信息报告脚本

# 操作系统版本
echo "操作系统信息:"
lsb_release -a

# 内核版本
echo -e "\n内核版本:"
uname -r

# CPU 信息
echo -e "\nCPU 信息:"
lscpu | grep "Model name" | uniq

# 内存使用情况
echo -e "\n内存使用情况:"
free -h

# 磁盘使用情况
echo -e "\n磁盘使用情况:"
df -h

# 网络接口状态
echo -e "\n网络接口状态:"
ip addr show

# 当前运行的服务
echo -e "\n当前运行的服务:"
systemctl list-units --type=service --state=running
