"""Keep new benchmark archives and large raw files out of Git history."""
import argparse
from pathlib import Path
import subprocess

LIMIT = 5 * 1024 * 1024


def violation(name, size):
    return name.lower().endswith('.zip') or size > LIMIT


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', default='HEAD')
    args = parser.parse_args()
    base = args.base if args.base and set(args.base) != {'0'} else 'HEAD^'
    changed = subprocess.check_output(['git', 'diff', '--name-only', '--diff-filter=AM', '-z', base, '--', 'docs/benchmarks/'])
    untracked = subprocess.check_output(['git', 'ls-files', '--others', '--exclude-standard', '-z', '--', 'docs/benchmarks/'])
    rejected = []
    for name in set((changed + untracked).decode('utf-8').split('\0')) - {''}:
        path = Path(name)
        if path.is_file() and violation(name, path.stat().st_size):
            rejected.append(name)
    if rejected:
        parser.exit(1, 'Store raw evidence under artifacts/ and upload a pinned Actions artifact:\n' + '\n'.join(sorted(rejected)) + '\n')


if __name__ == '__main__':
    main()
