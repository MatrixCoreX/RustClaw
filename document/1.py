#!/usr/bin/env python3

import time
import sys


def countdown(seconds):
    """Simple countdown timer that prints remaining time every second."""
    for i in range(seconds, 0, -1):
        print(f'{i} seconds remaining...')
        time.sleep(1)
    print('Time\'s up!')


if __name__ == '__main__':
    if len(sys.argv) != 2:
        print('Usage: python3 1.py <seconds>')
        sys.exit(1)
    
    try:
        sec = int(sys.argv[1])
        if sec <= 0:
            raise ValueError('Seconds must be a positive integer')
        countdown(sec)
    except ValueError as e:
        print(f'Error: {e}')
        sys.exit(1)
