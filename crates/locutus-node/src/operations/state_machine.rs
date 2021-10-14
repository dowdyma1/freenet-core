//! Inspired by rust-fsm. Brought in tree for modifying and tailoring it to
//! this application needs.

use super::OpError;

pub trait StateMachineImpl {
    /// The input alphabet.
    type Input;
    /// The set of possible states.
    type State;
    /// The output alphabet.
    type Output;

    /// The transition fuction that outputs a new state based on the current
    /// state and the provided input. Outputs `None` when there is no transition
    /// for a given combination of the input and the state.
    fn transition(state: Self::State, input: Self::Input) -> Option<Self::State>;

    /// The output function that outputs some value from the output alphabet
    /// based on the current state and the given input. Outputs `None` when
    /// there is no output for a given combination of the input and the state.
    fn output(state: &Self::State, input: &Self::Input) -> Option<Self::Output>;
}

/// A convenience wrapper around the `StateMachine` trait that encapsulates the
/// state and transition and output function calls.
pub(crate) struct StateMachine<T: StateMachineImpl> {
    state: Option<T::State>,
}

impl<T> StateMachine<T>
where
    T: StateMachineImpl,
{
    /// Create a new instance of this wrapper which encapsulates the given
    /// state.
    pub fn from_state(state: T::State) -> Self {
        Self { state: Some(state) }
    }

    /// Consumes the provided input, gives an output and performs a state
    /// transition. If a state transition with the current state and the
    /// provided input is not allowed, returns an error.
    pub fn consume<CErr>(&mut self, input: T::Input) -> Result<Option<T::Output>, OpError<CErr>> {
        let popped_state = self.state.take().expect("infallible");
        let output = T::output(&popped_state, &input);
        if let Some(state) = T::transition(popped_state, input) {
            self.state = Some(state);
            Ok(output)
        } else {
            Err(OpError::IllegalStateTransition)
        }
    }

    /// Returns the current state.
    pub fn state(&self) -> &T::State {
        self.state.as_ref().expect("infallible")
    }
}
