use std::ops::Index;

use super::BuilderError;

/// A fully-qualified name, used to identify a specific element in the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuilderFqn(Vec<String>);

impl BuilderFqn {
    pub fn append(mut self, name: &str) -> Self {
        self.0.push(name.to_string());
        self
    }

    pub fn pop(&mut self) -> Option<String> {
        self.0.pop()
    }

    pub fn peek(&self) -> Option<&str> {
        self.0.last().map(String::as_str)
    }

    /// Split the last element from the FQN, returning the new FQN and the last element.
    pub fn split_last(mut self) -> Option<(Self, String)> {
        self.0.pop().map(|last| (self, last))
    }
}

impl TryFrom<&str> for BuilderFqn {
    type Error = BuilderError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let inner = value.split("::").map(str::to_owned).collect::<Vec<_>>();
        if inner.is_empty() {
            Err(BuilderError::InvalidFqn(value.to_string()))
        } else {
            Ok(Self(inner))
        }
    }
}

impl std::fmt::Display for BuilderFqn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.join("::"))
    }
}

impl Index<usize> for BuilderFqn {
    type Output = String;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

#[test]
fn test_fqn() {
    let fqn = BuilderFqn::try_from("boomerang::builder::fqn").unwrap();
    assert_eq!(fqn.to_string(), "boomerang::builder::fqn");
    assert_eq!(fqn[0], "boomerang");
    assert_eq!(fqn[1], "builder");
    assert_eq!(fqn[2], "fqn");
}
