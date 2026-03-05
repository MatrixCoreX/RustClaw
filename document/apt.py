#!/usr/bin/env python3

import subprocess
import sys
import logging

# Configure logging
current_log = '/var/log/apt_upgrade.log'
logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(levelname)s - %(message)s',
    handlers=[
        logging.FileHandler(current_log),
        logging.StreamHandler(sys.stdout)
    ]
)


def run_cmd(cmd):
    """Run a shell command and return (returncode, stdout, stderr)"""
    try:
        result = subprocess.run(
            cmd,
            shell=True,
            capture_output=True,
            text=True,
            timeout=600  # 10 min timeout
        )
        return result.returncode, result.stdout, result.stderr
    except subprocess.TimeoutExpired as e:
        logging.error(f'Command timed out: {cmd}')
        return -1, '', str(e)
    except Exception as e:
        logging.error(f'Unexpected error running command {cmd}: {e}')
        return -1, '', str(e)


def main():
    logging.info('Starting Ubuntu system upgrade...')

    # Step 1: apt update
    logging.info('Running `apt update`...')
    code, out, err = run_cmd('sudo apt update')
    if code != 0:
        logging.error(f'`apt update` failed with exit code {code}')
        logging.error(f'STDERR: {err.strip()}')
        sys.exit(1)

    # Step 2: apt list --upgradable (optional check)
    logging.info('Checking for upgradable packages...')
    code, out, err = run_cmd('apt list --upgradable 2>/dev/null | tail -n +2 | wc -l')
    if code == 0 and out.strip().isdigit():
        upgradable_count = int(out.strip())
        logging.info(f'Found {upgradable_count} upgradable package(s).')
        if upgradable_count == 0:
            logging.info('No packages require upgrading. System is up to date.')
            return
    else:
        logging.warning('Could not determine upgradable count; proceeding to upgrade.')

    # Step 3: apt upgrade -y
    logging.info('Running `apt upgrade -y`...')
    code, out, err = run_cmd('sudo apt upgrade -y')
    if code == 0:
        logging.info('Ubuntu system upgrade completed successfully.')
    else:
        logging.error(f'`apt upgrade -y` failed with exit code {code}')
        logging.error(f'STDERR: {err.strip()}')
        sys.exit(1)


if __name__ == '__main__':
    main()
