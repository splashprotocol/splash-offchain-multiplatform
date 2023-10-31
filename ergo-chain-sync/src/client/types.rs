use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

use derive_more::{Display, Into};
use isahc::http::{uri::InvalidUri, Uri};
use serde::Deserialize;

#[derive(Debug, Clone, Into, Deserialize)]
#[serde(try_from = "String")]
pub struct Url(Uri);

impl TryFrom<String> for Url {
    type Error = InvalidUrl;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Url::from_str(&value)
    }
}

impl Display for Url {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Display)]
pub enum InvalidUrl {
    Uri(InvalidUri),
    NoScheme,
}

impl FromStr for Url {
    type Err = InvalidUrl;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uri = Uri::from_str(s).map_err(InvalidUrl::Uri)?;
        if uri.scheme_str().is_none() {
            return Err(InvalidUrl::NoScheme);
        }
        Ok(Url(uri))
    }
}

impl From<Url> for String {
    fn from(value: Url) -> Self {
        value.0.to_string()
    }
}

pub fn with_path(Url(uri): &Url, path: &str) -> Uri {
    isahc::http::uri::Builder::new()
        .scheme(uri.scheme_str().unwrap())
        .authority(uri.authority().unwrap().as_str())
        .path_and_query(path)
        .build()
        .unwrap()
}
