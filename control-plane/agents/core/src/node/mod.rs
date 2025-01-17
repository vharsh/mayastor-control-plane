mod registry;
/// Node Service
pub(super) mod service;
mod specs;
/// node watchdog to keep track of a node's liveness
pub(crate) mod watchdog;

use super::{controller::registry::Registry, CliArgs};
use common::Service;
use common_lib::{
    transport_api::{v0::*, *},
    types::v0::transport::{GetBlockDevices, GetNodes},
};
use grpc::operations::{node::server::NodeServer, registration::server::RegistrationServer};
use std::sync::Arc;

/// Configure the Service and return the builder.
pub(crate) async fn configure(builder: Service) -> Service {
    let node_service = create_node_service(&builder).await;
    let node_grpc_service = NodeServer::new(Arc::new(node_service.clone()));
    let registration_service = RegistrationServer::new(Arc::new(node_service.clone()));
    builder
        .with_shared_state(node_service)
        .with_shared_state(node_grpc_service)
        .with_shared_state(registration_service)
}

async fn create_node_service(builder: &Service) -> service::Service {
    let registry = builder.shared_state::<Registry>().clone();
    let deadline = CliArgs::args().deadline.into();
    let request = CliArgs::args().request_timeout.into();
    let connect = CliArgs::args().connect_timeout.into();
    let no_min = CliArgs::args().no_min_timeouts;

    service::Service::new(registry.clone(), deadline, request, connect, no_min).await
}
