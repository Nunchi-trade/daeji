//! HDC precompile provider wrapper.
//!
//! Wraps the standard `EthPrecompiles` to intercept calls to the HDC
//! precompile address (0x09) and delegate to `kora_hdc_chain::hdc_precompile`.

use revm::{
    context::Cfg,
    context_interface::ContextTr,
    handler::{EthPrecompiles, PrecompileProvider},
    interpreter::{CallInputs, Gas, InstructionResult, InterpreterResult},
    primitives::{Address, Bytes, hardfork::SpecId},
};

/// Precompile provider that intercepts calls to the HDC address (0x09)
/// and delegates all other addresses to the standard Ethereum precompiles.
#[derive(Debug, Clone)]
pub(crate) struct HdcPrecompileProvider {
    inner: EthPrecompiles,
}

impl HdcPrecompileProvider {
    /// Create a new HDC precompile provider wrapping the standard Ethereum precompiles.
    pub(crate) fn new(spec: SpecId) -> Self {
        Self { inner: EthPrecompiles::new(spec) }
    }
}

impl<CTX: ContextTr> PrecompileProvider<CTX> for HdcPrecompileProvider
where
    <CTX::Cfg as Cfg>::Spec: Into<SpecId>,
{
    type Output = InterpreterResult;

    fn set_spec(&mut self, spec: <CTX::Cfg as Cfg>::Spec) -> bool {
        <EthPrecompiles as PrecompileProvider<CTX>>::set_spec(&mut self.inner, spec)
    }

    fn run(
        &mut self,
        context: &mut CTX,
        inputs: &CallInputs,
    ) -> Result<Option<InterpreterResult>, String> {
        if inputs.bytecode_address == kora_hdc_chain::HDC_PRECOMPILE_ADDRESS {
            let input_bytes = inputs.input.as_bytes(context);
            let gas_limit = inputs.gas_limit;

            match kora_hdc_chain::hdc_precompile(&input_bytes, gas_limit) {
                Ok((gas_used, output)) => {
                    let mut gas = Gas::new(gas_limit);
                    let _ = gas.record_regular_cost(gas_used);
                    Ok(Some(InterpreterResult {
                        result: InstructionResult::Return,
                        gas,
                        output: Bytes::from(output),
                    }))
                }
                Err(kora_hdc_chain::precompile::PrecompileError::OutOfGas) => {
                    let mut gas = Gas::new(gas_limit);
                    gas.spend_all();
                    Ok(Some(InterpreterResult {
                        result: InstructionResult::PrecompileOOG,
                        gas,
                        output: Bytes::new(),
                    }))
                }
                Err(e) => {
                    let mut gas = Gas::new(gas_limit);
                    gas.spend_all();
                    Ok(Some(InterpreterResult {
                        result: InstructionResult::PrecompileError,
                        gas,
                        output: Bytes::from(e.to_string().into_bytes()),
                    }))
                }
            }
        } else {
            self.inner.run(context, inputs)
        }
    }

    fn warm_addresses(&self) -> Box<impl Iterator<Item = Address>> {
        // Replace BLAKE2F (0x09) from the standard set with HDC at the same address.
        // Filter out the inner 0x09 to avoid duplicates, then append HDC's 0x09.
        let hdc_addr = kora_hdc_chain::HDC_PRECOMPILE_ADDRESS;
        let inner_addrs = self.inner.warm_addresses();
        Box::new(inner_addrs.filter(move |a| *a != hdc_addr).chain(std::iter::once(hdc_addr)))
    }

    fn contains(&self, address: &Address) -> bool {
        *address == kora_hdc_chain::HDC_PRECOMPILE_ADDRESS || self.inner.contains(address)
    }
}
