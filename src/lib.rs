#![cfg_attr(not(feature = "std"), no_std)]

//! # A Concordium V1 smart contract
use concordium_std::collections::*;
use concordium_std::*;
use core::fmt::Debug;

/// How many of the owners need to agree before transfer
pub const TRANSFER_AGREEMENT_THRESHOLD: usize = 3;

// Types
pub type TransferRequestId = u128;

#[derive(Serialize, SchemaType, Clone)]
pub struct TransferRequest {
    pub transfer_amount: Amount,
    pub target_account: AccountAddress,
    pub supporters: BTreeSet<AccountAddress>,
}

///smart contract state.
#[derive(Serial, DeserialWithState)]
#[concordium(state_parameter = "S")]
pub struct State<S> {
    /// Who is authorized to sig (must be non-empty)
    pub owners: BTreeSet<AccountAddress>,

    ///The id assigned to last request
    pub last_request_id: TransferRequestId,

    /// Requests which have not been dropped due to timing out or due to being
    /// agreed to yet The request ID, the associated amount, when it times
    /// out, who is making the transfer and which account owners support
    /// this transfer
    pub requests: StateMap<TransferRequestId, TransferRequest, S>,
}

#[derive(Serialize, SchemaType, Clone)]
pub struct InitParams {
    /// Who is authorized to sig (must be non-empty)
    #[concordium(size_length = 1)]
    pub owners: BTreeSet<AccountAddress>,
}

#[derive(Serialize, SchemaType, Clone)]
pub struct SubmitParams {
    pub transfer_amount: Amount,
    pub target_account: AccountAddress,
}

#[derive(Debug, PartialEq, Eq, Reject, Serial, SchemaType)]
pub enum Error {
    /// Failed parsing the parameter.
    #[from(ParseError)]
    ParseParams,

    /// Not enough account owners.
    InsufficientOwners,
    /// Only account owners can interact with this contract.
    NotOwner,
    /// Sender cannot be a contract.
    ContractSender,
    /// Not enough available funds for the requested transfer.
    InsufficientAvailableFunds,
    /// Not such request funds.
    RequestNotFound,
    /// A request with this ID already exists.
    RequestAlreadyExists,
    /// Transfer amount or account is different from the request.
    MismatchingRequestInformation,
    /// You have already supported this transfer.
    RequestAlreadySupported,
    /// You have not already supported this transfer.
    RequestAlreadyNotSupported,
    /// All owners have not supported the request
    RequestNotSupportedByAllOwners,

    /// Invalid receiver when invoking a transfer.
    InvokeTransferMissingAccount,
    /// Insufficient funds when invoking a transfer.
    InvokeTransferInsufficientFunds,
}

/// Mapping errors related to transfer invocations to CustomContractError.
impl From<TransferError> for Error {
    fn from(te: TransferError) -> Self {
        match te {
            TransferError::AmountTooLarge => Self::InvokeTransferInsufficientFunds,
            TransferError::MissingAccount => Self::InvokeTransferMissingAccount,
        }
    }
}

fn is_owner(account: Address, owners: &BTreeSet<AccountAddress>) -> bool {
    owners.iter().any(|owner| account.matches_account(owner))
}

// Contract implementation
//--------------- contract functions ----------
#[init(contract = "multisig_wallet", parameter = "InitParams", payable)]
#[inline(always)]
pub fn contract_init<S: HasStateApi>(
    ctx: &impl HasInitContext,
    state_builder: &mut StateBuilder<S>,
    _amount: Amount,
) -> Result<State<S>, Error> {
    let init_params: InitParams = ctx.parameter_cursor().get()?;
    let owners = init_params.owners;
    ensure!(
        owners.len() == TRANSFER_AGREEMENT_THRESHOLD,
        Error::InsufficientOwners
    );

    let state = State {
        owners,
        last_request_id: 0,
        requests: state_builder.new_map(),
    };

    Ok(state)
}

#[receive(contract = "multisig_wallet", name = "deposit", payable)]
fn contract_receive_deposit<S: HasStateApi>(
    _ctx: &impl HasReceiveContext,
    _host: &impl HasHost<State<S>, StateApiType = S>,
    _amount: Amount,
) -> ReceiveResult<()> {
    Ok(())
}

#[receive(
    contract = "multisig_wallet",
    name = "submit_transfer_request",
    parameter = "SubmitParams",
    mutable,
    error = "Error"
)]
pub fn contract_receive_submit_transfer_request<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> Result<TransferRequestId, Error> {
    let sender = ctx.sender();
    let owners = &host.state().owners;

    ensure!(is_owner(sender, owners), Error::NotOwner);

    let sender_address = match sender {
        Address::Contract(_) => bail!(Error::ContractSender),
        Address::Account(account_address) => account_address,
    };

    let submit_params: SubmitParams = ctx.parameter_cursor().get()?;

    let req_id = host.state().last_request_id + 1;
    let transfer_amount = submit_params.transfer_amount;
    let target_account = submit_params.target_account;

    let mut supporters = BTreeSet::new();
    supporters.insert(sender_address);

    let new_request = TransferRequest {
        transfer_amount,
        target_account,
        supporters,
    };

    host.state_mut().requests.insert(req_id, new_request);
    host.state_mut().last_request_id = req_id;

    Ok(req_id)
}

#[receive(
    contract = "multisig_wallet",
    name = "support_transfer_request",
    parameter = "TransferRequestId",
    mutable,
    error = "Error"
)]
pub fn contract_receive_support_transfer_request<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> Result<(), Error> {
    let sender = ctx.sender();
    let owners = &host.state().owners;

    ensure!(is_owner(sender, owners), Error::NotOwner);

    let sender_address = match sender {
        Address::Contract(_) => bail!(Error::ContractSender),
        Address::Account(account_address) => account_address,
    };

    let request_id: TransferRequestId = ctx.parameter_cursor().get()?;

    let mut matching_request = host
        .state_mut()
        .requests
        .entry(request_id)
        .occupied_or(Error::RequestNotFound)?;

    ensure!(
        !matching_request.supporters.contains(&sender_address),
        Error::RequestAlreadySupported
    );
    matching_request.supporters.insert(sender_address);

    Ok(())
}

#[receive(
    contract = "multisig_wallet",
    name = "not_support_transfer_request",
    parameter = "TransferRequestId",
    mutable,
    error = "Error"
)]
pub fn contract_receive_not_support_transfer_request<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> Result<(), Error> {
    let sender = ctx.sender();
    let owners = &host.state().owners;

    ensure!(is_owner(sender, owners), Error::NotOwner);

    let sender_address = match sender {
        Address::Contract(_) => bail!(Error::ContractSender),
        Address::Account(account_address) => account_address,
    };

    let request_id: TransferRequestId = ctx.parameter_cursor().get()?;

    let mut matching_request = host
        .state_mut()
        .requests
        .entry(request_id)
        .occupied_or(Error::RequestNotFound)?;

    ensure!(
        matching_request.supporters.contains(&sender_address),
        Error::RequestAlreadyNotSupported
    );
    matching_request.supporters.remove(&sender_address);

    Ok(())
}

#[receive(
    contract = "multisig_wallet",
    name = "execute_transfer_request",
    parameter = "TransferRequestId",
    mutable,
    error = "Error"
)]
pub fn contract_receive_execute_transfer_request<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> Result<(), Error> {
    let sender = ctx.sender();
    let owners = &host.state().owners;

    ensure!(is_owner(sender, owners), Error::NotOwner);

    let request_id: TransferRequestId = ctx.parameter_cursor().get()?;

    match host.state().requests.get(&request_id) {
        None => Err(Error::RequestNotFound),
        Some(matching_request) => {
            ensure!(
                !matching_request.supporters.len() == TRANSFER_AGREEMENT_THRESHOLD,
                Error::RequestNotSupportedByAllOwners
            );
            let target_account = matching_request.target_account;
            let transfer_amount = matching_request.transfer_amount;

            host.state_mut().requests.remove(&request_id);
            host.invoke_transfer(&target_account, transfer_amount)?;

            Ok(())
        }
    }
}

#[receive(
    contract = "multisig_wallet",
    name = "view_transfer_request",
    parameter = "TransferRequestId",
    mutable,
    return_value = "TransferRequest",
    error = "Error"
)]
pub fn contract_receive_view_transfer_request<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> Result<TransferRequest, Error> {
    let sender = ctx.sender();
    let owners = &host.state().owners;

    ensure!(is_owner(sender, owners), Error::NotOwner);

    let request_id: TransferRequestId = ctx.parameter_cursor().get()?;

    match host.state().requests.get(&request_id) {
        None => Err(Error::RequestNotFound),
        Some(matching_request) => {
            ensure!(
                !matching_request.supporters.len() == TRANSFER_AGREEMENT_THRESHOLD,
                Error::RequestNotSupportedByAllOwners
            );
            Ok(matching_request.clone())
        }
    }
}
