#!/usr/bin/env python3
import automerge
import json

def dump(doc: automerge.Document):
    if isinstance(doc, automerge.Mapping):
        res = {}
        for name, value in automerge.entries(doc):
            if isinstance(value, automerge.Document):
                value = dump(value)
            elif isinstance(value, automerge.Counter):
                value = value.get()
            elif isinstance(value, automerge.Text):
                value = str(value)
            res[name] = value
    else: # sequence
        res = []
        for value in doc:
            if isinstance(value, automerge.Document):
                value = dump(value)
            elif isinstance(value, automerge.Counter):
                value = value.get()
            elif isinstance(value, automerge.Text):
                value = str(value)
            res.append(value)

    return res

def dd(doc):
    print(json.dumps(dump(doc), indent=4))

doc = automerge.init()

with automerge.transaction(doc) as d:
    d.hello = {"hello": [{"a":1, "2":3}]}
    # d.riea = [1,2,3,4,5, "riea"]
    d.riea = [1,2,3,4,5]
    d.riea[:1] = [1,2,34]
    d.riea[::3] = [1,2,34]



# print(doc.riea[::3])
# for k in doc:
#     print(k)

dd(doc)


with automerge.transaction(doc) as d:
    d.hello.hello[0] = False # = {"hello": [{"a":1, "2":3}]}

    # d.riea.insert(1, 3)

dd(doc)


data = automerge.save(doc)

doc2 = automerge.load(data)

with automerge.transaction(doc) as d:
    d.hello.hello[4:] = [1,3,4]

with automerge.transaction(doc2) as d:
    d.hello.hello[:0] = [1]

dd(doc)
dd(doc2)

automerge.merge(doc, doc2)

dd(doc)

change = automerge.get_last_local_change(doc).bytes()
print(change)

automerge.apply_changes(doc2, [change])
dd(doc2)


doc3 = automerge.fork(doc)

with automerge.transaction(doc3) as d:
    d.text = automerge.Text("hello")
    d.counter = automerge.Counter(123)


doc_a = automerge.fork(doc3)
doc_b = automerge.fork(doc3)

with automerge.transaction(doc_a) as d:
    d.text[:0] = "Somebody says: "
    d.counter.increment(100)

with automerge.transaction(doc_b) as d:
    d.text[10000:] = ", World! :)"
    d.counter.increment(10)

dd(doc_a)
dd(doc_b)

automerge.merge(doc_a, doc_b)

dd(doc_a)

from typing import Dict, List

class Idea:
    name: str
    priority: float


class MyDoc:
    ideas: List[Idea]

ideas_doc = automerge.init(MyDoc)

with automerge.transaction(ideas_doc) as d:
    d.ideas = [{"name": "a idea", "priority": 1.0}]
    d.ideas[0].name = "other name"


dd(ideas_doc)
