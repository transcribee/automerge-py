# Python Frontend for Automerge

The Python frontend for [automerge-rs](https://github.com/automerge/automerge-rs)

## Build

```sh
# Install maturin
pip install maturin
# Create venv in "env" folder (required by maturin)
python3 -m venv env
# Activate venv
source ./env/bin/activate

# Build automerge_backend (Python bindings to Rust) and install it as a Python module
maturin develop
```

## Usage

See `test.py`
