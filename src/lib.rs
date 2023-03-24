use std::sync::{Arc, Mutex};

use automerge::{
    transaction::{CommitOptions, Transactable, Transaction, UnObserved},
    Automerge, ChangeHash, ObjId, ObjType, Prop, ReadDoc, ScalarValue, Value,
};
use log;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyMapping, PySequence, PySlice};
use pyo3::{
    exceptions::{PyException, PyIndexError, PyTypeError, PyValueError},
    types::PyString,
};
use pyo3_log;
use std::convert::TryInto;
use tracing;
use tracing_subscriber;

// The document type
// This has shared ownership between all instances of Documents with the same underlying Automerge Document.
// The python Document can refer to any of the Maps or Lists inside the Automerge Document
// Mutex is needed because we support multithreading from the python side
// The Option is needed to be able to move the Automerge Document into the struct holding the transaction
// (as the transaction needs a mutable reference to the document)
type AutomergeDocument = Arc<Mutex<Option<Automerge>>>;

// the baseclass for the python bindings for a Automerge Document.
// Each instance can refere to one of the Maps or Lists inside the Document
// It provides access to the items or properties of that List or Map
#[pyclass(subclass)]
pub struct Document {
    obj_id: ObjId,
    automerge: AutomergeDocument,
}

impl Document {
    fn from_doc(py: Python<'_>, doc: Automerge) -> PyResult<PyObject> {
        Document::for_subfield_inner(
            py,
            None,
            Arc::new(Mutex::new(Some(doc))),
            ObjType::Map,
            automerge::ROOT,
        )
    }

    fn for_subfield(
        py: Python<'_>,
        doc: &Automerge,
        automerge: AutomergeDocument,
        ty: ObjType,
        obj_id: ObjId,
    ) -> PyResult<PyObject> {
        Document::for_subfield_inner(py, Some(doc), automerge, ty, obj_id)
    }

    fn for_subfield_inner(
        py: Python<'_>,
        // We only need this in the text case. This allows both from_doc and for_subfield to use this function
        // from_doc will never hit the text case
        document: Option<&Automerge>,
        automerge: AutomergeDocument,
        ty: ObjType,
        obj_id: ObjId,
    ) -> PyResult<PyObject> {
        let doc = Self {
            obj_id: obj_id.clone(),
            automerge,
        };
        Ok(match ty {
            ObjType::Map | ObjType::Table => {
                let init = PyClassInitializer::from(doc).add_subclass(Mapping);
                PyCell::new(py, init)?.to_object(py)
            }
            ObjType::List => {
                let init = PyClassInitializer::from(doc).add_subclass(Sequence);
                PyCell::new(py, init)?.to_object(py)
            }
            ObjType::Text => {
                // TODO(robin): this feels a bit unclean
                // maybe we want three text types or so?
                // Text for input, Text when reading and Text for Transaction?
                let document = document.unwrap();
                PyCell::new(
                    py,
                    Text {
                        text: document
                            .text(obj_id.clone())
                            .map_err(AutomergeError::AutomergeError)?,
                    },
                )?
                .to_object(py)
            }
        })
    }
}

macro_rules! with_doc {
    ($self:ident, |$doc:ident| $func:tt) => {{
        let automerge = $self.automerge.lock().unwrap();
        let $doc = automerge
            .as_ref()
            .ok_or(AutomergeError::UsingDocDuringTransaction)?;
        $func
    }};
}

macro_rules! with_doc_mut {
    ($self:ident, |$doc:ident| $func:tt) => {{
        let mut automerge = $self.automerge.lock().unwrap();
        let $doc = automerge
            .as_mut()
            .ok_or(AutomergeError::UsingDocDuringTransaction)?;
        $func
    }};
}

#[pymethods]
impl Document {
    fn __len__(&self) -> PyResult<usize> {
        with_doc! {self, |doc| {
            Ok(doc.length(self.obj_id.clone()))
        }}
    }
    fn dump(&self) -> PyResult<()> {
        with_doc! {self, |doc| {
            Ok(doc.dump())
        }}
    }
}

// converts a automerge value to the appropriate python value
fn read_value<'a, T: ReadDoc>(
    py: Python<'_>,
    doc: &T,
    obj_id: ObjId,
    name: impl Into<IndexOrName<'a>>,
    nested_handler: impl FnOnce(ObjType, ObjId) -> PyResult<PyObject>,
    counter_handler: Option<impl FnOnce() -> PyResult<PyObject>>,
) -> PyResult<PyObject> {
    match doc
        .get(obj_id.clone(), name.into())
        .map_err(AutomergeError::AutomergeError)?
    {
        Some((Value::Object(ty), id)) => nested_handler(ty, id),
        Some((Value::Scalar(s), _)) => {
            use ScalarValue::*;
            let s = &*s;
            Ok(match s {
                Bytes(b) => b.to_object(py),
                Str(s) => s.to_object(py),
                Int(i) => i.to_object(py),
                Uint(i) => i.to_object(py),
                F64(f) => f.to_object(py),
                Counter(c) => {
                    if let Some(counter_handler) = counter_handler {
                        counter_handler()?
                    } else {
                        crate::Counter(c.into()).into_py(py)
                    }
                }
                // TODO(robin): this probably should become a date?
                Timestamp(t) => t.to_object(py),
                Boolean(b) => b.to_object(py),
                Unknown { type_code, bytes } => crate::Unknown {
                    type_code: *type_code,
                    bytes: bytes.to_vec(),
                }
                .into_py(py),
                Null => ().to_object(py),
            })
        }
        None => Ok(().to_object(py)),
    }
}

#[derive(FromPyObject)]
enum IndexOrName<'a> {
    Int(usize),
    String(&'a str),
}

impl<'a> From<IndexOrName<'a>> for automerge::Prop {
    fn from(idx_or_name: IndexOrName<'a>) -> Self {
        match idx_or_name {
            IndexOrName::Int(i) => i.into(),
            IndexOrName::String(s) => s.into(),
        }
    }
}

impl<'a> From<&'a str> for IndexOrName<'a> {
    fn from(s: &'a str) -> Self {
        IndexOrName::String(s)
    }
}

impl<'a> From<&'a String> for IndexOrName<'a> {
    fn from(s: &'a String) -> Self {
        IndexOrName::String(&*s)
    }
}

impl<'a> From<usize> for IndexOrName<'a> {
    fn from(i: usize) -> Self {
        IndexOrName::Int(i)
    }
}

// special sub class for mappings
#[pyclass(extends=Document, mapping)]
pub struct Mapping;

// special sub class for sequences
#[pyclass(extends=Document, sequence)]
pub struct Sequence;

#[pymethods]
impl Mapping {
    fn __getitem__(slf: PyRef<'_, Self>, py: Python<'_>, name: &'_ str) -> PyResult<PyObject> {
        Mapping::__getattr__(slf, py, name)
    }

    fn __getattr__(slf: PyRef<'_, Self>, py: Python<'_>, name: &'_ str) -> PyResult<PyObject> {
        let super_ = slf.as_ref();
        with_doc! {super_, |doc| {
            read_value(py, doc, super_.obj_id.clone(), name, |ty, obj_id| {
                Document::for_subfield(py, doc, super_.automerge.clone(), ty, obj_id)
            }, Option::<fn() -> _>::None)
        }}
    }
}

// TODO(robin): consider implementing the sequence iterator on our own?
// Maybe thats faster...
#[pymethods]
impl Sequence {
    fn __getitem__(slf: PyRef<'_, Self>, py: Python<'_>, mut index: isize) -> PyResult<PyObject> {
        let super_ = slf.as_ref();
        with_doc! {super_, |doc| {
            let length = doc.length(super_.obj_id.clone());
            if index < 0 {
                let isize_length: isize = length.try_into().unwrap();
                index = index + isize_length;
            }
            if index < 0 {
                return Err(PyIndexError::new_err(format!("index out of range")))
            }
            let index: usize = index.try_into().unwrap();
            if index < length {
                read_value(py, doc, super_.obj_id.clone(), index, |ty, obj_id| {
                    Ok(Document::for_subfield(py, doc, super_.automerge.clone(), ty, obj_id)?.into_py(py))
                }, Option::<fn() -> _>::None)
            } else {
                Err(PyIndexError::new_err(format!("index {index} is greater than length {length}")))
            }
        }}
    }
}

// fn __setitem__(&self) {
// }

// fn __delitem__(&self) {
// }

#[pyclass]
pub struct EntriesIterator {
    automerge: AutomergeDocument,
    obj_id: ObjId,
    keys: std::vec::IntoIter<String>,
}

#[pymethods]
impl EntriesIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(
        mut slf: PyRefMut<'_, Self>,
        py: Python<'_>,
    ) -> PyResult<Option<(String, PyObject)>> {
        let key = slf.keys.next();
        Ok(match key {
            Some(key) => {
                let value = with_doc! {slf, |doc| {
                    read_value(py, doc, slf.obj_id.clone(), &key, |ty, obj_id| {
                        Ok(Document::for_subfield(py, doc, slf.automerge.clone(), ty, obj_id)?.into_py(py))
                    }, Option::<fn() -> _>::None)?
                }};
                Some((key, value))
            }
            None => None,
        })
    }
}

// We need use a standalone function, because pyo3 does not support returning
// a subclass from the constructor
// and manually overriding __new__ does not seem to be supported
// It has a additional argument to allow passing a "type", which
// is completely faken in the .pyi files
#[pyfunction]
pub fn init(py: Python<'_>, _ignore: Option<&PyAny>) -> PyResult<PyObject> {
    Document::from_doc(py, Automerge::new())
}

// TODO(robin): check for Sequence. Currently returns empty iterator for sequence
// TODO(robin): is there a way to not read all the keys at once?
#[pyfunction]
pub fn entries(document: &mut Document) -> PyResult<EntriesIterator> {
    let keys = with_doc! {document, |doc| {
        doc.keys(document.obj_id.clone()).collect::<Vec<_>>()
    }};
    Ok(EntriesIterator {
        keys: keys.into_iter(),
        obj_id: document.obj_id.clone(),
        automerge: document.automerge.clone(),
    })
}

#[pyfunction]
pub fn transaction(
    py: Python<'_>,
    doc: &mut Document,
    message: Option<String>,
) -> PyResult<PyObject> {
    let automerge = doc
        .automerge
        .lock()
        .unwrap()
        .take()
        .ok_or(AutomergeError::NestedTransaction)?;
    DocumentTransaction::new(py, automerge, doc, message)
}

// TODO(robin): Support observers. Currently we don't support observers
type Tx<'a> = Transaction<'a, UnObserved>;

// The transaction needs a mutable reference to the Document.
// To stick the transaction into a struct and export it to python we need a self referential struct
// that holds both the document and the transaction, to guarantee the document lives atleast as long as the transaction
#[ouroboros::self_referencing]
#[derive(Debug)]
struct TransactionOwningDocument {
    owner: Automerge,
    #[borrows(mut owner)]
    #[covariant]
    transaction: Option<Tx<'this>>,
}
type TransactionHolder = Option<TransactionOwningDocument>;

// Python class providing bindigs to transactions. This again works similar to Document and can refer to any of the Maps or Lists inside the Automerge Document
#[pyclass(subclass)]
#[derive(Clone, Debug)]
pub struct DocumentTransaction {
    automerge: AutomergeDocument,
    transaction: Arc<Mutex<TransactionHolder>>,
    obj_id: ObjId,
    commit_message: Option<String>,
    change_hash: Option<ChangeHash>,
}
impl DocumentTransaction {
    fn new(
        py: Python<'_>,
        automerge: Automerge,
        document: &Document,
        commit_message: Option<String>,
    ) -> PyResult<PyObject> {
        let ty = automerge
            .object_type(document.obj_id.clone())
            .map_err(AutomergeError::AutomergeError)?;
        DocumentTransaction::for_subfield(
            py,
            document.automerge.clone(),
            Arc::new(Mutex::new(Some(
                TransactionOwningDocumentBuilder {
                    owner: automerge,
                    transaction_builder: |owner| Some(owner.transaction()),
                }
                .build(),
            ))),
            ty,
            document.obj_id.clone(),
            commit_message,
        )
    }

    fn for_subfield(
        py: Python<'_>,
        automerge: AutomergeDocument,
        transaction: Arc<Mutex<TransactionHolder>>,
        ty: ObjType,
        obj_id: ObjId,
        commit_message: Option<String>,
    ) -> PyResult<PyObject> {
        let doc = Self {
            automerge,
            transaction,
            obj_id,
            commit_message,
            change_hash: None,
        };
        match ty {
            ObjType::Map | ObjType::Table => {
                let init = PyClassInitializer::from(doc).add_subclass(MappingTransaction);
                Ok(PyCell::new(py, init)?.to_object(py))
            }
            ObjType::List => {
                let init = PyClassInitializer::from(doc).add_subclass(SequenceTransaction);
                Ok(PyCell::new(py, init)?.to_object(py))
            }
            ObjType::Text => {
                let init = PyClassInitializer::from(doc).add_subclass(TextTransaction);
                Ok(PyCell::new(py, init)?.to_object(py))
            }
        }
    }

    fn __str__(&self) -> String {
        format!("{:?}", self)
    }
}

macro_rules! with_transaction {
    ($self:ident, |$tx:ident| $func:tt) => {
        let mut tx = $self.transaction.lock().unwrap();
        let tx = tx.as_mut().ok_or(AutomergeError::ReusedTransaction)?;
        Ok(tx.with_transaction_mut(|tx| {
            let $tx = tx.as_mut().unwrap();
            Result::<_, PyErr>::Ok($func?)
        })?)
    };
}

#[pymethods]
impl DocumentTransaction {
    // TODO(robin): maybe split out these?
    fn __enter__(slf: PyRef<'_, DocumentTransaction>) -> PyResult<PyRef<'_, DocumentTransaction>> {
        if slf.transaction.lock().unwrap().is_none() {
            Err(AutomergeError::ReusedTransaction)?
        } else {
            Ok(slf)
        }
    }

    fn __exit__(&mut self, ty: Option<&PyAny>, _value: &PyAny, _traceback: &PyAny) -> PyResult<()> {
        let mut tx = self
            .transaction
            .lock()
            .unwrap()
            .take()
            .ok_or(AutomergeError::ReusedTransaction)?;
        if ty.is_none() {
            tx.with_transaction_mut(|tx| {
                let tx = tx.take().unwrap();
                if let Some(msg) = &self.commit_message {
                    self.change_hash = tx.commit_with(CommitOptions::default().with_message(msg));
                    tracing::trace!(?self.change_hash, "commiting tx");
                } else {
                    self.change_hash = tx.commit();
                    tracing::trace!(?self.change_hash, "commiting tx");
                }
            });
        }

        // not commiting automatically rolls back
        let heads = tx.into_heads();
        *self.automerge.lock().unwrap() = Some(heads.owner);
        Ok(())
    }

    fn __len__(&self) -> PyResult<usize> {
        with_transaction! {self, |tx| {
            PyResult::Ok(tx.length(self.obj_id.clone()))
        }}
    }

    fn get_change(&self) -> PyResult<Option<Change>> {
        if let Some(hash) = self.change_hash {
            with_doc!(self, |doc| {
                PyResult::Ok(doc.get_change_by_hash(&hash).map(|change| Change {
                    change: change.clone(),
                }))
            })
        } else {
            PyResult::Ok(None)
        }
    }
}

// special sub class for transactions on counters
#[pyclass(extends=DocumentTransaction)]
pub struct CounterTransaction {
    prop: Prop,
}

impl CounterTransaction {
    fn new(
        py: Python<'_>,
        base: &DocumentTransaction,
        prop: impl Into<Prop>,
    ) -> PyResult<PyObject> {
        let init = PyClassInitializer::from(base.clone())
            .add_subclass(CounterTransaction { prop: prop.into() });
        Ok(PyCell::new(py, init)?.to_object(py))
    }
}

// TODO(robin): prevent this from having __len__?
#[pymethods]
impl CounterTransaction {
    fn increment(mut slf: PyRefMut<'_, Self>, py: Python<'_>, increment: i64) -> PyResult<()> {
        let prop = slf.prop.clone();
        let super_ = slf.as_mut();
        let obj_id = super_.obj_id.clone();
        with_transaction! {super_, |tx| {
            tx.increment(obj_id, prop, increment).map_err(AutomergeError::AutomergeError)
        }}
    }
}

// special sub class for transactions on mappings
#[pyclass(extends=DocumentTransaction, mapping)]
pub struct MappingTransaction;

#[pymethods]
impl MappingTransaction {
    fn __getitem__(slf: PyRefMut<'_, Self>, py: Python<'_>, name: &'_ str) -> PyResult<PyObject> {
        MappingTransaction::__getattr__(slf, py, name)
    }

    fn __getattr__(
        mut slf: PyRefMut<'_, Self>,
        py: Python<'_>,
        name: &'_ str,
    ) -> PyResult<PyObject> {
        let super_ = slf.as_mut();
        with_transaction! {super_, |tx| {
            read_value(py, tx, super_.obj_id.clone(), name, |ty, obj_id| {
                DocumentTransaction::for_subfield(py, super_.automerge.clone(), super_.transaction.clone(), ty, obj_id, None)
            },
            Some(|| CounterTransaction::new(py, super_, name))
            )
        }}
    }

    fn __setitem__(
        slf: PyRefMut<'_, Self>,
        name: &'_ str,
        value: AutomergeValue<'_>,
    ) -> PyResult<()> {
        MappingTransaction::__setattr__(slf, name, value)
    }

    fn __setattr__(
        mut slf: PyRefMut<'_, Self>,
        name: &'_ str,
        value: AutomergeValue<'_>,
    ) -> PyResult<()> {
        let super_ = slf.as_mut();
        with_transaction! {super_, |tx| {
            apply_value(tx, super_.obj_id.clone(), name, value)
        }}
    }

    fn __delitem__(slf: PyRefMut<'_, Self>, name: &'_ str) -> PyResult<()> {
        MappingTransaction::__delattr__(slf, name)
    }

    fn __delattr__(mut slf: PyRefMut<'_, Self>, name: &'_ str) -> PyResult<()> {
        let super_ = slf.as_mut();
        with_transaction! {super_, |tx| {
            tx.delete(super_.obj_id.clone(), name).map_err(AutomergeError::AutomergeError)
        }}
    }
}

#[derive(FromPyObject)]
enum SliceOrIndex<'a> {
    Index(usize),
    Slice(&'a PySlice),
}

// special sub class for transactions on sequences
#[pyclass(extends=DocumentTransaction, sequence)]
pub struct SequenceTransaction;

// TODO(robin): make a sequence look more like an actual list
// by implemnting
// - append, clear, extend, index, count, insert, pop, remove, reverse?
#[pymethods]
impl SequenceTransaction {
    fn __getitem__(
        slf: PyRefMut<'_, Self>,
        py: Python<'_>,
        mut index: isize,
    ) -> PyResult<PyObject> {
        let super_ = slf.as_ref();
        with_transaction! {super_, |tx| {
            let length = tx.length(super_.obj_id.clone());
            if index < 0 {
                let isize_length: isize = length.try_into().unwrap();
                index = index + isize_length;
            }
            if index < 0 {
                return Err(PyIndexError::new_err(format!("index out of range")))
            }
            let index: usize = index.try_into().unwrap();
            if index < length {
                read_value(py, tx, super_.obj_id.clone(), index, |ty, obj_id| {
                    Ok(DocumentTransaction::for_subfield(py, super_.automerge.clone(), super_.transaction.clone(), ty, obj_id, None)?.into_py(py))
                },
                Some(|| CounterTransaction::new(py, super_, index))
                )
            } else {
                Err(PyIndexError::new_err(format!("index {index} is greater than length {length}")))
            }
        }}
    }

    fn __setitem__(
        mut slf: PyRefMut<'_, Self>,
        index_or_slice: SliceOrIndex<'_>,
        value: AutomergeValue<'_>,
    ) -> PyResult<()> {
        let super_ = slf.as_mut();
        with_transaction! {super_, |tx| {
            match index_or_slice {
                SliceOrIndex::Index(index) => {
                    let length = tx.length(super_.obj_id.clone());
                    if index == length { // Setting the n+1'th item is the same as appending, so we add a dummy element
                        tx.splice(super_.obj_id.clone(), length, 0, [ScalarValue::Null]).map_err(AutomergeError::AutomergeError)?;
                    }
                    Ok(apply_value(tx, super_.obj_id.clone(), index, value)?)
                },
                SliceOrIndex::Slice(slice) => {
                    let length = tx.length(super_.obj_id.clone());
                    let slice = slice.indices(length as _)?;

                    match value {
                        AutomergeValue::Sequence(s) => {
                            let sequence_len = s.len()?;
                            if slice.step != 1 && (slice.slicelength as usize) != sequence_len {
                                Err(PyValueError::new_err(
                                    format!("attempt to assign sequence of size {} to extended slice of size {}", sequence_len, slice.slicelength)
                                ))
                            } else {

                                // for step == 1, we "replace" the old slice with a new sequence and the lenght could change
                                // we insert dummy values for the new sequence
                                // for step != 1, we simply replace the values
                                if slice.step == 1 {
                                    tx.splice(super_.obj_id.clone(), slice.start as usize, slice.slicelength as usize, std::iter::repeat(ScalarValue::Null).take(sequence_len)).map_err(AutomergeError::AutomergeError)?;
                                }

                                // now simply write the values
                                for (i, elem) in s.iter()?.enumerate() {
                                    let i = (slice.start + (i as isize) * slice.step) as usize;
                                    apply_value(tx, super_.obj_id.clone(), i, elem?.extract()?)?;
                                }
                                Ok(())

                            }

                        }
                        _ => Err(PyTypeError::new_err(
                            format!("can only assign an iterable")
                        ))
                    }
                }
            }
        }}
    }

    fn __delitem__(mut slf: PyRefMut<'_, Self>, index: usize) -> PyResult<()> {
        let super_ = slf.as_mut();
        with_transaction! {super_, |tx| {
            tx.delete(super_.obj_id.clone(), index).map_err(AutomergeError::AutomergeError)
        }}
    }

    fn append(mut slf: PyRefMut<'_, Self>, value: AutomergeValue<'_>) -> PyResult<()> {
        let super_ = slf.as_mut();
        with_transaction! {super_, |tx| {
                let length = tx.length(super_.obj_id.clone());
                // Setting the n+1'th item is the same as appending, so we add a dummy element
                tx.splice(super_.obj_id.clone(), length, 0, [ScalarValue::Null]).map_err(AutomergeError::AutomergeError)?;
                apply_value(tx, super_.obj_id.clone(), length, value)
            }
        }
    }
}

// special sub class for transactions on Text
#[pyclass(extends=DocumentTransaction, sequence)]
pub struct TextTransaction;

#[pymethods]
impl TextTransaction {
    fn __getitem__(slf: PyRefMut<'_, Self>, py: Python<'_>, index: usize) -> PyResult<String> {
        let super_ = slf.as_ref();
        with_transaction! {super_, |tx| {
            let length = tx.length(super_.obj_id.clone());
            if index < length {
                Ok(tx.get(super_.obj_id.clone(), index).map_err(AutomergeError::AutomergeError)?.unwrap().0.into_string().unwrap())
            } else {
                Err(PyIndexError::new_err(format!("index {index} is greater than length {length}")))
            }
        }}
    }

    fn __setitem__(
        mut slf: PyRefMut<'_, Self>,
        index_or_slice: SliceOrIndex<'_>,
        value: &str,
    ) -> PyResult<()> {
        let super_ = slf.as_mut();
        with_transaction! {super_, |tx| {
            let value_len = value.chars().count();
            match index_or_slice {
                SliceOrIndex::Index(index) => {
                    // TODO(robin): do we get unicode length mismatch here?
                    // (python str.len() vs automerge str length)
                    // also python index vs rust index
                    Ok(tx.splice_text(super_.obj_id.clone(), index, value_len, value).map_err(AutomergeError::AutomergeError)?)
                },
                SliceOrIndex::Slice(slice) => {
                    let length = tx.length(super_.obj_id.clone());
                    let slice = slice.indices(length as _)?;

                    // TODO(robin): same here
                    if slice.step != 1 && (slice.slicelength as usize) != value_len {
                        Err(PyValueError::new_err(
                            format!("attempt to assign sequence of size {} to extended slice of size {}", value_len, slice.slicelength)
                        ))
                    } else {
                        // for step == 1, we "replace" the old slice with a new sequence and the lenght could change
                        // we insert dummy values for the new sequence
                        // for step != 1, we simply replace the values
                        if slice.step == 1 {
                            tx.splice_text(
                                super_.obj_id.clone(),
                                slice.start as usize,
                                slice.slicelength as usize,
                                value
                            ).map_err(AutomergeError::AutomergeError)?;
                        } else {
                            let mut buffer = [0u8; 4];
                            // write the values
                            for (i, elem) in value.chars().enumerate() {
                                let i = (slice.start + (i as isize) * slice.step) as usize;
                                tx.splice_text(
                                    super_.obj_id.clone(),
                                    i,
                                    1,
                                    elem.encode_utf8(&mut buffer)
                                ).map_err(AutomergeError::AutomergeError)?
                            }
                        }

                        Ok(())
                    }
                }
            }
        }}
    }

    fn __delitem__(mut slf: PyRefMut<'_, Self>, index: usize) -> PyResult<()> {
        let super_ = slf.as_mut();
        with_transaction! {super_, |tx| {
            tx.splice_text(super_.obj_id.clone(), index, 1, "").map_err(AutomergeError::AutomergeError)
        }}
    }
}

macro_rules! match_value {
    ($value:expr,
        Scalar($scalar:ident) => $scalar_handler:tt,
        Sequence($sequence:ident) => $sequence_handler:tt,
        Mapping($mapping:ident) => $mapping_handler:tt,
        Text($text:ident) => $text_handler:tt,
    ) => {
        use AutomergeValue::*;
        match_value!(
            @gen_arms, $value, $scalar, $scalar_handler, Bytes, Str, Int, Uint, F64, Counter, Boolean : rest, {
                match_value!(@gen_arms, rest, $sequence, $sequence_handler, Sequence : rest, {
                    match_value!(@gen_arms, rest, $mapping, $mapping_handler, Mapping : rest, {
                        match_value!(@gen_arms, rest, $text, $text_handler, Text : _rest, {
                            unreachable!();
                        })
                    })
                })
            }
        )
    };
    (@gen_arms, $value:expr, $name:ident, $handler:tt, $($cases:ident),* : $other:ident, $rest_handler:tt) => {
        match $value {
            $($cases($name) => $handler)*,
            $other => $rest_handler,
        }
    }
}

#[derive(FromPyObject, Debug)]
struct PyBytesNT<'a>(&'a PyBytes);

impl<'a> From<PyBytesNT<'a>> for ScalarValue {
    fn from(bytes: PyBytesNT<'a>) -> Self {
        ScalarValue::Bytes(bytes.0.as_bytes().to_vec())
    }
}

// TODO(robin): allow arbitrary things and use .__dict__?
// These are the values we support for conversion into Automerge values
#[derive(Debug, FromPyObject)]
enum AutomergeValue<'a> {
    Boolean(bool),
    Str(&'a str),
    Int(i64),
    Uint(u64),
    F64(f64),
    Counter(Counter),
    Text(&'a PyCell<Text>),
    Bytes(PyBytesNT<'a>),
    Mapping(&'a PyMapping),
    Sequence(&'a PySequence),
}

// This converts from a python value to a Automerge value and creates the appropriate transaction to write that value to the document
fn apply_value(
    tx: &mut Tx,
    obj: impl AsRef<ObjId>,
    prop: impl Into<Prop>,
    value: AutomergeValue,
) -> Result<(), PyErr> {
    match_value!(value,
        Scalar(s) => {
            tx.put(obj, prop, s).map_err(AutomergeError::AutomergeError)?;
        },
        Sequence(s) => {
            // TODO(robin): sequence creation could be optimized:
            // 1. remove the dummy splice by adding a flag to apply_value to do insertion instead of puts
            // 2. replace the dummy splice with a real splice if all values are ScalarValues
            let sequence_id = tx.put_object(obj, prop, ObjType::List).map_err(AutomergeError::AutomergeError)?;
            // insert dummy values for all new entries in the list
            tx.splice(sequence_id.clone(), 0, 0, std::iter::repeat(ScalarValue::Null).take(s.len()?)).map_err(AutomergeError::AutomergeError)?;
            for (i, elem) in s.iter()?.enumerate() {
                apply_value(tx, sequence_id.clone(), i, elem?.extract()?)?;
            }
        },
        Mapping(m) => {
            let mapping_id = tx.put_object(obj, prop, ObjType::Map).map_err(AutomergeError::AutomergeError)?;
            for entry in m.items()?.iter()? {
                let (name, elem): (&str, AutomergeValue) = entry?.extract()?;
                apply_value(tx, mapping_id.clone(), name, elem)?;
            }
        },
        Text(t) => {
            let text_id = tx.put_object(obj, prop, ObjType::Text).map_err(AutomergeError::AutomergeError)?;
            // overwrite the complete text
            tx.splice_text(text_id, 0, 0, &t.borrow().text).map_err(AutomergeError::AutomergeError)?;
        },
    );

    Ok(())

    // put
    // put_object
    // insert
    // insert_object
    // increment
    // delete
    // splice
    // splice_text
}

// special class for unknown automerge values
#[pyclass]
struct Unknown {
    type_code: u8,
    bytes: Vec<u8>,
}

// special class for the automerge Text value which is basically a List that only supports unicode codepoints as values
#[pyclass]
#[derive(Debug)]
struct Text {
    text: String,
}

#[pymethods]
impl Text {
    #[new]
    fn new(text: String) -> Self {
        Self { text }
    }

    fn __str__(&self) -> String {
        self.text.clone()
    }
}

// special class for automerge Counters, which support incremeting
#[pyclass]
#[derive(Clone, Debug)]
struct Counter(i64);

#[pymethods]
impl Counter {
    #[new]
    fn new(value: i64) -> Self {
        Self(value)
    }

    fn get(&self) -> i64 {
        self.0
    }
}

impl From<Counter> for ScalarValue {
    fn from(counter: Counter) -> ScalarValue {
        ScalarValue::Counter(counter.0.into())
    }
}

#[pyfunction]
pub fn fork(py: Python<'_>, doc: &Document) -> PyResult<PyObject> {
    let new_doc = with_doc!(doc, |doc| { doc.fork() });

    Document::from_doc(py, new_doc)
}

#[pyfunction]
pub fn merge(doc_a: &mut Document, doc_b: &mut Document) -> PyResult<()> {
    Ok(with_doc_mut!(doc_a, |doc_a| {
        with_doc_mut!(doc_b, |doc_b| {
            doc_a.merge(doc_b).map_err(AutomergeError::AutomergeError)?;
        })
    }))
}

#[pyfunction]
pub fn save(py: Python<'_>, doc: &mut Document) -> PyResult<Py<PyBytes>> {
    Ok(with_doc_mut!(doc, |doc| {
        PyBytes::new(py, &doc.save()[..]).into()
    }))
}

#[pyfunction]
pub fn load(py: Python<'_>, bytes: &PyBytes) -> PyResult<PyObject> {
    let new_doc = Automerge::load(bytes.as_bytes()).map_err(AutomergeError::AutomergeError)?;
    Document::from_doc(py, new_doc)
}

#[pyclass]
#[derive(Clone)]
pub struct Change {
    change: automerge::Change,
}

#[pymethods]
impl Change {
    #[new]
    fn new(bytes: &PyBytes) -> PyResult<Self> {
        Ok(Self {
            change: automerge::Change::from_bytes(bytes.as_bytes().to_vec())
                .map_err(AutomergeError::LoadChangeError)?,
        })
    }

    fn bytes(&mut self, py: Python<'_>) -> Py<PyBytes> {
        PyBytes::new(py, &*self.change.bytes()).into()
    }

    fn decode(&mut self, py: Python<'_>) -> PyResult<ExpandedChange> {
        Ok(ExpandedChange {
            change: self.change.decode(),
        })
    }
}

#[pyclass]
#[derive(Debug, Clone)]
pub struct ExpandedChange {
    change: automerge::ExpandedChange,
}

#[pymethods]
impl ExpandedChange {
    fn __repr__(&self) -> String {
        format!("{:?}", self)
    }
}

#[pyfunction]
pub fn apply_changes(doc: &mut Document, changes: &PySequence) -> PyResult<()> {
    Ok(with_doc_mut!(doc, |doc| {
        for change in changes.iter()? {
            let change = change?;
            let change = if let Ok(change) = change.downcast::<PyBytes>() {
                automerge::Change::from_bytes(change.as_bytes().to_vec())
                    .map_err(AutomergeError::LoadChangeError)?
            } else {
                Change::extract(change)?.change
            };
            doc.apply_changes(std::iter::once(change))
                .map_err(AutomergeError::AutomergeError)?;
        }
    }))
}

#[pyfunction]
pub fn get_last_local_change(doc: &Document) -> PyResult<Option<Change>> {
    Ok(with_doc!(doc, |doc| {
        doc.get_last_local_change().map(|change| Change {
            change: change.clone(),
        })
    }))
}

#[derive(Debug)]
pub enum AutomergeError {
    NestedTransaction,
    ReusedTransaction,
    UsingDocDuringTransaction,
    AutomergeError(automerge::AutomergeError),
    LoadChangeError(automerge::LoadChangeError),
}

impl From<AutomergeError> for PyErr {
    fn from(error: AutomergeError) -> Self {
        match error {
            AutomergeError::NestedTransaction => {
                PyValueError::new_err("nested transactions are not allowed")
            }
            AutomergeError::ReusedTransaction => {
                PyValueError::new_err("transaction was already commited, cannot use it again")
            }
            AutomergeError::UsingDocDuringTransaction => {
                PyValueError::new_err("document used while there is a uncommited transaction")
            }
            AutomergeError::AutomergeError(e) => {
                PyException::new_err(format!("Automerge error: {}", e))
            }
            AutomergeError::LoadChangeError(e) => {
                PyValueError::new_err(format!("LoadChangeError error: {}", e))
            }
        }
    }
}

// impl From<automerge::AutomergeError> for PyErr {
//     fn from(error: automerge::AutomergeError) -> Self {
//         PyException::new_err(error)
//     }
// }

#[pymodule]
fn _backend(_py: Python, m: &PyModule) -> PyResult<()> {
    tracing_subscriber::fmt::init();

    m.add_class::<Document>()?;
    m.add_class::<Mapping>()?;
    m.add_class::<Sequence>()?;
    m.add_class::<DocumentTransaction>()?;
    m.add_class::<MappingTransaction>()?;
    m.add_class::<SequenceTransaction>()?;
    m.add_class::<Change>()?;
    m.add_class::<Text>()?;
    m.add_class::<Counter>()?;
    m.add_function(wrap_pyfunction!(transaction, m)?)?;
    m.add_function(wrap_pyfunction!(entries, m)?)?;
    m.add_function(wrap_pyfunction!(init, m)?)?;
    m.add_function(wrap_pyfunction!(fork, m)?)?;
    m.add_function(wrap_pyfunction!(merge, m)?)?;
    m.add_function(wrap_pyfunction!(load, m)?)?;
    m.add_function(wrap_pyfunction!(save, m)?)?;
    m.add_function(wrap_pyfunction!(apply_changes, m)?)?;
    m.add_function(wrap_pyfunction!(get_last_local_change, m)?)?;
    Ok(())
}
