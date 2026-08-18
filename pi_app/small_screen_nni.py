import urllib.parse
from datetime import datetime


def nni_runtime_is_active(config):
    return bool(isinstance(config, dict) and config.get("joined") and config.get("worker_running", True))


def _node_host(config):
    if not isinstance(config, dict):
        return "--"
    host = str(config.get("last_success_node_host") or "").strip()
    if host:
        return host
    node_url = str(config.get("selected_node_url") or "").strip()
    if not node_url:
        nodes = config.get("remote_nodes")
        if isinstance(nodes, list) and nodes:
            node_url = str(nodes[0] or "").strip()
    try:
        return urllib.parse.urlparse(node_url).hostname or "--"
    except ValueError:
        return "--"


def _clock_label(timestamp):
    if not isinstance(timestamp, (int, float)) or timestamp <= 0:
        return "--"
    try:
        return datetime.fromtimestamp(timestamp).strftime("%m-%d %H:%M")
    except (OverflowError, OSError, ValueError):
        return "--"


def format_nni_runtime_summary(config, device, lang="CN", error=""):
    selected_lang = "EN" if lang == "EN" else "CN"
    if not isinstance(config, dict) or not config:
        if error:
            return "NNI status unavailable" if selected_lang == "EN" else "NNI 状态暂时无法读取"
        return "Loading NNI status..." if selected_lang == "EN" else "正在读取 NNI 状态..."

    copy = {
        "CN": {
            "status": "状态",
            "running": "运行中",
            "stopped": "未启动",
            "heartbeat": "心跳",
            "active": "正常",
            "enabling": "启动中",
            "waiting_network": "等待网络",
            "degraded": "异常",
            "rejected": "已拒绝",
            "disabled": "未启用",
            "chip": "芯片",
            "hardware": "硬件",
            "simulated": "模拟",
            "unavailable": "不可用",
            "node": "节点",
            "authorization": "授权",
            "authorized": "已授权",
            "unknown": "待确认",
            "requests": "请求",
            "latest": "最近",
            "sync_error": "状态同步失败，保留上次结果",
        },
        "EN": {
            "status": "Status",
            "running": "Running",
            "stopped": "Stopped",
            "heartbeat": "Heartbeat",
            "active": "Active",
            "enabling": "Starting",
            "waiting_network": "Waiting for network",
            "degraded": "Degraded",
            "rejected": "Rejected",
            "disabled": "Disabled",
            "chip": "Chip",
            "hardware": "Hardware",
            "simulated": "Simulated",
            "unavailable": "Unavailable",
            "node": "Node",
            "authorization": "Authorization",
            "authorized": "Authorized",
            "unknown": "Pending",
            "requests": "Requests",
            "latest": "Latest",
            "sync_error": "Status refresh failed; showing the last result",
        },
    }[selected_lang]

    active = nni_runtime_is_active(config)
    heartbeat_state = str(config.get("heartbeat_state") or ("enabling" if active else "disabled"))
    heartbeat_label = copy.get(heartbeat_state, heartbeat_state)
    device = device if isinstance(device, dict) else {}
    if device.get("hardware_chip_present") or (
        device.get("signature_chip_present") and not device.get("simulated")
    ):
        chip_label = copy["hardware"]
    elif device.get("simulated"):
        chip_label = copy["simulated"]
    else:
        chip_label = copy["unavailable"]
    authorization = str(config.get("network_authorization") or "unknown")
    authorization_label = copy.get(authorization, authorization)
    request_count = config.get("heartbeat_request_count")
    if not isinstance(request_count, int) or isinstance(request_count, bool):
        request_count = 0

    lines = [
        f"{copy['status']}: {copy['running'] if active else copy['stopped']}  ·  "
        f"{copy['heartbeat']}: {heartbeat_label}  ·  {copy['chip']}: {chip_label}",
        f"{copy['node']}: {_node_host(config)}  ·  {copy['authorization']}: {authorization_label}",
        f"{copy['requests']}: {request_count}  ·  "
        f"{copy['latest']}: {_clock_label(config.get('last_heartbeat_at_ts'))}",
    ]
    if error:
        lines.append(copy["sync_error"])
    return "\n".join(lines)
