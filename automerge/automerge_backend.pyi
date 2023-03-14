#!/usr/bin/env python3

from typing import *


T = TypeVar('T')

class Document(Generic[T]):
    ...

def init(type: Type[T]) -> Document[T]:
    ...


class DocumentTransaction(Generic[T]):
    def __enter__(self) -> T:
        ...

    def __exit__(self, _, __, ___):
        ...

def transaction(doc: Document[T]) -> DocumentTransaction[T]:
    ...
