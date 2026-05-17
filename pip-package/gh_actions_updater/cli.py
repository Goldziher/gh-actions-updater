import sys

from .downloader import run_binary


def main():
    run_binary(sys.argv[1:])


if __name__ == "__main__":
    main()
