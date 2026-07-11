//! Axum routes for ontology schema resources.

mod error;
mod instance_routes;
mod schema_routes;

use std::sync::Arc;

use axum::Router;
use wyse_ontology::OntologyService;

use crate::instance_routes::instance_routes;
use crate::schema_routes::schema_routes;

/// Shared state for ontology schema routes.
#[derive(Clone)]
pub struct AppState {
    pub(crate) service: Arc<OntologyService>,
}

/// Builds the ontology REST router without binding a network listener.
pub fn router(service: Arc<OntologyService>) -> Router {
    Router::new()
        .nest("/v1", instance_routes())
        .nest("/v1/ontology", schema_routes())
        .with_state(AppState { service })
}
