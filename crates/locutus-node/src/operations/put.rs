//! A contract is PUT within a location distance, this entails that all nodes within
//! a given radius will cache a copy of the contract and it's current value,
//! as well as will broadcast updates to the contract value to all subscribers.

use std::time::Duration;

use rust_fsm::{StateMachine, StateMachineImpl};

use crate::{
    config::PEER_TIMEOUT_SECS,
    conn_manager::{ConnectionBridge, PeerKeyLocation},
    contract::{Contract, ContractError, ContractHandlerEvent},
    message::{GetTxType, Message, Transaction},
    node::{OpExecError, OpManager},
    ring::{Location, RingError},
};

pub(crate) use self::messages::PutMsg;

use super::{handle_op_result, OpError, Operation, OperationResult};

pub(crate) type ContractPutValue = Vec<u8>;

pub(crate) struct PutOp {
    sm: StateMachine<PutOpSM>,
    /// time left until time out, when this reaches zero it will be removed from the state
    _ttl: Duration,
}

impl PutOp {
    pub fn start_op(contract: Contract, value: Vec<u8>, htl: usize) -> Self {
        log::debug!(
            "Requesting put to contract {} @ loc({})",
            contract.key(),
            Location::from(contract.key())
        );
        let id = Transaction::new(<PutMsg as GetTxType>::tx_type_id());
        let sm = StateMachine::from_state(PutState::Requesting {
            id,
            contract,
            value,
            htl,
        });
        PutOp {
            sm,
            _ttl: Duration::from_secs(PEER_TIMEOUT_SECS),
        }
    }
}

struct PutOpSM;

impl StateMachineImpl for PutOpSM {
    type Input = PutMsg;

    type State = PutState;

    type Output = PutMsg;

    const INITIAL_STATE: Self::State = PutState::Initializing;

    fn transition(state: &Self::State, input: &Self::Input) -> Option<Self::State> {
        match (state, input) {
            // state changed for the initial requesting node
            (PutState::Requesting { contract, .. }, PutMsg::RouteValue { .. }) => {
                Some(PutState::AwaitAnswer {
                    contract: contract.clone(),
                })
            }
            (PutState::AwaitAnswer { contract, .. }, PutMsg::SuccessfulUpdate { .. }) => {
                log::debug!("Successfully updated value for {}", contract.key());
                None
            }
            // state changes for the target node
            (
                PutState::Initializing | PutState::BroadcastOngoing { .. },
                PutMsg::Broadcasting {
                    broadcast_to,
                    broadcasted_to,
                    ..
                },
            ) => {
                if *broadcasted_to >= broadcast_to.len() {
                    // broadcast complete
                    None
                } else {
                    Some(PutState::BroadcastOngoing {
                        left_peers: broadcast_to.clone(),
                        completed: *broadcasted_to,
                    })
                }
            }
            // state changes for proxies
            (PutState::Initializing, PutMsg::PutProxy { .. }) => None,
            _ => None,
        }
    }

    fn output(state: &Self::State, input: &Self::Input) -> Option<Self::Output> {
        match (state, input) {
            (
                PutState::Requesting {
                    contract,
                    value,
                    id,
                    htl,
                },
                PutMsg::RouteValue { .. },
            ) => Some(PutMsg::RequestPut {
                id: *id,
                contract: contract.clone(),
                value: value.clone(),
                htl: *htl,
            }),
            (PutState::Initializing, PutMsg::Broadcasting { id, new_value, .. }) => {
                Some(PutMsg::SuccessfulUpdate {
                    id: *id,
                    new_value: new_value.clone(),
                })
            }
            _ => None,
        }
    }
}

enum PutState {
    Initializing,
    Requesting {
        id: Transaction,
        contract: Contract,
        value: ContractPutValue,
        htl: usize,
    },
    AwaitAnswer {
        contract: Contract,
    },
    BroadcastOngoing {
        left_peers: Vec<PeerKeyLocation>,
        completed: usize,
    },
}

impl PutState {
    fn is_requesting(&self) -> bool {
        matches!(self, Self::Requesting { .. })
    }

    fn id(&self) -> &Transaction {
        todo!()
    }
}

/// Request to insert/update a value into a contract.
pub(crate) async fn request_put<CErr>(
    op_storage: &OpManager<CErr>,
    mut put_op: PutOp,
) -> Result<(), OpError<CErr>> {
    if !put_op.sm.state().is_requesting() {
        return Err(OpError::IllegalStateTransition);
    };

    if let Some(req_put) = put_op.sm.consume(&PutMsg::RouteValue {
        id: *put_op.sm.state().id(),
        htl: op_storage.ring.max_hops_to_live,
    })? {
        op_storage
            .notify_change(Message::from(req_put), Operation::Put(put_op))
            .await?;
    } else {
        return Err(OpError::IllegalStateTransition);
    }
    Ok(())
}

pub(crate) async fn handle_put_response<CB, CErr>(
    op_storage: &OpManager<CErr>,
    conn_manager: &mut CB,
    put_op: PutMsg,
) -> Result<(), OpError<CErr>>
where
    CB: ConnectionBridge,
    OpError<CErr>: From<ContractError<CErr>>,
{
    let sender;
    let tx = *put_op.id();
    let result = match op_storage.pop(put_op.id()) {
        Some(Operation::Put(state)) => {
            sender = put_op.sender().cloned();
            // was an existing operation, the other peer messaged back
            update_state(conn_manager, state, put_op, op_storage).await
        }
        Some(_) => return Err(OpExecError::TxUpdateFailure(tx).into()),
        None => {
            sender = put_op.sender().cloned();
            // new request to join from this node, initialize the machine
            let machine = PutOp {
                sm: StateMachine::new(),
                _ttl: Duration::from_millis(PEER_TIMEOUT_SECS),
            };
            update_state(conn_manager, machine, put_op, op_storage).await
        }
    };

    handle_op_result(
        op_storage,
        conn_manager,
        result.map_err(|err| (err, tx)),
        sender,
    )
    .await
}

async fn update_state<CB, CErr>(
    conn_manager: &mut CB,
    mut state: PutOp,
    other_host_msg: PutMsg,
    op_storage: &OpManager<CErr>,
) -> Result<OperationResult, OpError<CErr>>
where
    CB: ConnectionBridge,
    OpError<CErr>: From<ContractError<CErr>>,
{
    let return_msg;
    let new_state;
    match other_host_msg {
        PutMsg::RequestPut {
            id,
            contract,
            value,
            htl,
        } => {
            // find the closest node to the location of the contract
            let target = if let Some(potential_target) = op_storage
                .ring
                .routing(&contract.key().location(), 1)
                .into_iter()
                .next()
            {
                potential_target
            } else {
                return Err(RingError::EmptyRing.into());
            };
            // the initial request must provide:
            // - a peer as close as possible to the contract location
            // - and the value to put

            return_msg = Some(
                (PutMsg::SeekNode {
                    id,
                    target,
                    sender: op_storage
                        .ring
                        .own_location()
                        .map(|location| PeerKeyLocation {
                            location: Some(location),
                            peer: conn_manager.peer_key(),
                        })
                        .ok_or_else(|| {
                            <OpError<CErr> as From<RingError>>::from(RingError::NoLocationAssigned)
                        })?,
                    contract,
                    value,
                    htl,
                })
                .into(),
            );
            // no changes to state yet, still in AwaitResponse state
            new_state = Some(state);
        }
        PutMsg::SeekNode {
            id,
            sender,
            value,
            contract,
            htl,
            target,
        } => {
            let key = contract.key();
            let cached_contract = op_storage.ring.has_contract(&key);
            if !cached_contract && op_storage.ring.within_caching_distance(&key.location()) {
                // this node does not have the contract, so instead store the contract and execute the put op.
                op_storage
                    .notify_contract_handler(ContractHandlerEvent::Cache(contract.clone()))
                    .await?;
            } else {
                // in this case forward to a closest node to the target location and just wait for a response
                // to give back to requesting peer
                todo!()
            }

            let new_value;
            // after the contract has been cached, push the update query
            match op_storage
                .notify_contract_handler(ContractHandlerEvent::PushQuery { key, value })
                .await
            {
                Ok(ContractHandlerEvent::PushResponse {
                    new_value: Ok(new_val),
                }) => {
                    new_value = new_val;
                }
                Ok(ContractHandlerEvent::PushResponse {
                    new_value: Err(_err),
                }) => {
                    // return Err(OpError::from(ContractError::StorageError(err)));
                    todo!("not a valid value update, notify back to requestor")
                }
                Err(err) => return Err(err.into()),
                Ok(_) => return Err(OpError::IllegalStateTransition),
            }

            // if the change was successful, communicate this back to the requestor and broadcast the change
            conn_manager
                .send(
                    &sender,
                    (PutMsg::SuccessfulUpdate {
                        id,
                        new_value: new_value.clone(),
                    })
                    .into(),
                )
                .await?;
            // TODO: actual broadcasting to subscribers of this contract
            let broadcast_to = op_storage.ring.closest_caching(&key, 10);

            // forward changes in the contract to nodes closer to the contract location, if possible
            let forward_to = broadcast_to.clone();
            let own_loc = op_storage.ring.own_location().expect("infallible");
            let contract_loc = key.location();
            for peer in &forward_to {
                let other_loc = peer.location.as_ref().expect("infallible");
                let other_distance = contract_loc.distance(other_loc);
                let self_distance = contract_loc.distance(&own_loc);
                if other_distance < self_distance {
                    // forward the contract towards this node since it is indeed closer
                    // to the contract location

                    // TODO: cloning the contract repeatedly is alloc heavy and costly performance wise
                    // may want to have a ref friendly method to pass array refs instead in CM
                    conn_manager
                        .send(
                            peer,
                            (PutMsg::PutProxy {
                                id,
                                contract: contract.clone(),
                                new_value: new_value.clone(),
                                htl: htl - 1,
                            })
                            .into(),
                        )
                        .await?;
                }
            }

            log::debug!(
                "Successfully updated a value for contract {} @ {:?}",
                contract.key(),
                target.location
            );

            return_msg = state
                .sm
                .consume(&PutMsg::Broadcasting {
                    id,
                    broadcasted_to: 0,
                    broadcast_to,
                    new_value,
                })?
                .map(Message::from);
            new_state = Some(state);
        }
        PutMsg::Broadcasting { .. } => {
            // here just keep updating the number of broadcasts done and whether broadcasting should be cancelled
            todo!()
        }
        PutMsg::SuccessfulUpdate { id, new_value } => {
            state
                .sm
                .consume(&PutMsg::SuccessfulUpdate { id, new_value })?;
            return_msg = None;
            new_state = None;
        }
        PutMsg::PutProxy { .. } => {
            // should here directly insert the update or run the value throught the value?
            todo!()
        }
        _ => return Err(OpError::IllegalStateTransition),
    }
    Ok(OperationResult {
        return_msg,
        state: new_state.map(Operation::Put),
    })
}

mod messages {
    use crate::conn_manager::PeerKeyLocation;

    use super::*;

    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub(crate) enum PutMsg {
        /// Initialize the put operation by routing the value
        RouteValue { id: Transaction, htl: usize },
        /// Internal node instruction to find a route to the target node.
        RequestPut {
            id: Transaction,
            contract: Contract,
            value: ContractPutValue,
            /// max hops to live
            htl: usize,
        },
        PutProxy {
            id: Transaction,
            contract: Contract,
            new_value: ContractPutValue,
            /// current htl, reduced by one at each hop
            htl: usize,
        },
        /// Value successfully inserted/updated.
        SuccessfulUpdate {
            id: Transaction,
            new_value: ContractPutValue,
        },
        /// Target the node which is closest to the key
        SeekNode {
            id: Transaction,
            sender: PeerKeyLocation,
            target: PeerKeyLocation,
            value: ContractPutValue,
            contract: Contract,
            /// max hops to live
            htl: usize,
        },
        /// Broadcast a change (either a first time insert or an update).
        Broadcasting {
            id: Transaction,
            broadcasted_to: usize,
            broadcast_to: Vec<PeerKeyLocation>,
            new_value: ContractPutValue,
        },
    }

    impl PutMsg {
        pub fn id(&self) -> &Transaction {
            match self {
                Self::SeekNode { id, .. } => id,
                Self::RouteValue { id, .. } => id,
                Self::RequestPut { id, .. } => id,
                Self::Broadcasting { id, .. } => id,
                Self::SuccessfulUpdate { id, .. } => id,
                Self::PutProxy { id, .. } => id,
            }
        }

        pub fn sender(&self) -> Option<&PeerKeyLocation> {
            match self {
                Self::SeekNode { sender, .. } => Some(sender),
                _ => None,
            }
        }

        pub fn target(&self) -> Option<&PeerKeyLocation> {
            match self {
                Self::SeekNode { target, .. } => Some(target),
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn successful_put_op_seq() -> Result<(), Box<dyn std::error::Error>> {
        let bytes = crate::test_utils::random_bytes_1024();
        let mut gen = arbitrary::Unstructured::new(&bytes);
        let contract: Contract = gen.arbitrary().map_err(|_| "failed gen arb data")?;

        let mut requester = PutOp::start_op(contract, vec![0, 1, 2, 3], 0).sm;
        let mut target = StateMachine::<PutOpSM>::new();
        Ok(())
    }
}