use serde::Serialize;

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Every error dbcore returns. Serialized to the UI as `{ kind, message }`.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    /// The server rejected something; `message` is the server's own text, shown verbatim.
    #[error("{message}")]
    Database {
        message: String,
        /// Engine error code (SQLSTATE, MySQL errno, Mongo code) when known.
        code: Option<String>,
        /// The statement that failed, when the error came from a script or change set.
        statement: Option<String>,
    },
    #[error("connection failed: {0}")]
    Connection(String),
    #[error("SSH tunnel: {0}")]
    Ssh(String),
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Unsupported(String),
    #[error("{0}")]
    InvalidInput(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("I/O: {0}")]
    Io(String),
    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn database(message: impl Into<String>) -> Self {
        Error::Database {
            message: message.into(),
            code: None,
            statement: None,
        }
    }

    /// Attaches the failing statement to a `Database` error.
    pub fn with_statement(self, sql: &str) -> Self {
        match self {
            Error::Database { message, code, .. } => Error::Database {
                message,
                code,
                statement: Some(sql.to_owned()),
            },
            other => other,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Error::Database { .. } => "database",
            Error::Connection(_) => "connection",
            Error::Ssh(_) => "ssh",
            Error::Cancelled => "cancelled",
            Error::Unsupported(_) => "unsupported",
            Error::InvalidInput(_) => "invalidInput",
            Error::NotFound(_) => "notFound",
            Error::Io(_) => "io",
            Error::Other(_) => "other",
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::Io(err.to_string())
    }
}

impl From<sqlx::Error> for Error {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::Database(db) => Error::Database {
                message: db.message().to_owned(),
                code: db.code().map(|c| c.into_owned()),
                statement: None,
            },
            sqlx::Error::Io(e) => Error::Connection(e.to_string()),
            sqlx::Error::Tls(e) => Error::Connection(format!("TLS: {e}")),
            sqlx::Error::PoolTimedOut => {
                Error::Connection("timed out waiting for a connection".into())
            }
            other => Error::Other(other.to_string()),
        }
    }
}

impl From<mongodb::error::Error> for Error {
    fn from(err: mongodb::error::Error) -> Self {
        Error::Database {
            message: err.to_string(),
            code: None,
            statement: None,
        }
    }
}

impl Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("Error", 4)?;
        st.serialize_field("kind", self.kind())?;
        st.serialize_field("message", &self.to_string())?;
        let (code, statement) = match self {
            Error::Database {
                code, statement, ..
            } => (code.clone(), statement.clone()),
            _ => (None, None),
        };
        st.serialize_field("code", &code)?;
        st.serialize_field("statement", &statement)?;
        st.end()
    }
}
