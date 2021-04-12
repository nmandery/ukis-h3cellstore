use pyo3::exceptions::{PyIOError, PyRuntimeError, PyValueError};
use pyo3::PyResult;

use bamboo_h3_int::error::Error;

/// convert the result of some other crate into a PyResult
pub trait IntoPyResult<T> {
    fn into_pyresult(self) -> PyResult<T>;
}

impl<T> IntoPyResult<T> for Result<T, bamboo_h3_int::error::Error> {
    fn into_pyresult(self) -> PyResult<T> {
        match self {
            Ok(v) => Ok(v),
            Err(err) => match err {
                Error::EmptyIndexes
                | Error::InvalidH3Index(_)
                | Error::MixedResolutions
                | Error::MissingQueryPlaceholder(_)
                | Error::InvalidH3Resolution(_)
                | Error::DifferentColumnLength(_, _, _)
                | Error::SchemaValidationError(_, _) => Err(PyValueError::new_err(err.to_string())),
                Error::NoQueryableTables
                | Error::SerializationError(_)
                | Error::ColumnNotFound(_)
                | Error::Clickhouse(_)
                | Error::H3ron(_)
                | Error::UnknownDatatype(_) => Err(PyRuntimeError::new_err(err.to_string())),
            },
        }
    }
}

impl<T> IntoPyResult<T> for bamboo_h3_int::clickhouse_rs::errors::Result<T> {
    fn into_pyresult(self) -> PyResult<T> {
        match self {
            Ok(v) => Ok(v),
            Err(err) => Err(PyIOError::new_err(err.to_string())),
        }
    }
}

impl<T> IntoPyResult<T> for Result<T, url::ParseError> {
    fn into_pyresult(self) -> PyResult<T> {
        match self {
            Ok(v) => Ok(v),
            Err(err) => Err(PyValueError::new_err(format!(
                "Invalid URL given: {}",
                err.to_string()
            ))),
        }
    }
}

impl<T> IntoPyResult<T> for std::io::Result<T> {
    fn into_pyresult(self) -> PyResult<T> {
        match self {
            Ok(v) => Ok(v),
            Err(err) => Err(PyIOError::new_err(format!("IO error: {}", err.to_string()))),
        }
    }
}

impl<T> IntoPyResult<T> for Result<T, h3ron::Error> {
    fn into_pyresult(self) -> PyResult<T> {
        match self {
            Ok(v) => Ok(v),
            // TODO: more fine-grained mapping to pyhton exceptions
            Err(err) => Err(PyValueError::new_err(err.to_string())),
        }
    }
}
