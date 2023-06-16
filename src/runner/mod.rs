#[cfg(not(feature = "federated"))]
mod non_federated;
#[cfg(not(feature = "federated"))]
pub use non_federated::*;

#[cfg(feature = "federated")]
mod federated;
#[cfg(feature = "federated")]
pub use federated::*;
