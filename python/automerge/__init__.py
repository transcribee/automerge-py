#!/usr/bin/env python3


from ._backend import (
    Document,
    Mapping,
    Sequence,
    transaction,
    entries,
    init,
    load,
    save,
    fork,
    merge,
    Change,
    apply_changes,
    get_last_local_change,
    Counter,
    Text,
)

__all__ = [
    "Document",
    "Mapping",
    "Sequence",
    "transaction",
    "entries",
    "init",
    "load",
    "save",
    "fork",
    "merge",
    "Change",
    "apply_changes",
    "get_last_local_change",
    "Counter",
    "Text",
]


def dump(doc: Document):
    if isinstance(doc, Mapping):
        res = {}
        for name, value in entries(doc):
            if isinstance(value, Document):
                value = dump(value)
            elif isinstance(value, Counter):
                value = value.get()
            elif isinstance(value, Text):
                value = str(value)
            res[name] = value
    else:  # sequence
        res = []
        for value in doc:
            if isinstance(value, Document):
                value = dump(value)
            elif isinstance(value, Counter):
                value = value.get()
            elif isinstance(value, Text):
                value = str(value)
            res.append(value)

    return res
