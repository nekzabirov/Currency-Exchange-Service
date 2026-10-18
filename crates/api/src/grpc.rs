//! The gRPC door. Thin: parse, find the registry row, compute, answer.

use std::sync::Arc;

use chrono::Utc;
use currency_core::logging::{call_span, TRACE_HEADER};
use currency_core::money::{self, RateError};
use currency_core::{ErrorCode, ERROR_CODE_KEY};
use tonic::{Request, Response, Status};

use crate::pb::currency_service_server::{CurrencyService, CurrencyServiceServer};
use crate::pb::{
    AmountResponse, ConvertFromUnitsRequest, ConvertToUnitsRequest, ConvertUnitsRequest,
    UnitsResponse,
};
use crate::rates::{Entry, Registry};

pub struct CurrencyApi {
    registry: Arc<Registry>,
}

impl CurrencyApi {
    pub fn new(registry: Arc<Registry>) -> Self {
        Self { registry }
    }

    pub fn into_server(self) -> CurrencyServiceServer<Self> {
        CurrencyServiceServer::new(self)
    }

    /// A registry row or a refusal.
    ///
    /// A stale rate is NOT rejected here: it is served, and a WARN with its age is left behind.
    /// Refusing would fail a payment because somebody else's API is down; computing silently would
    /// leave an investigation for a day on which everything "works" but the numbers are wrong.
    fn entry(&self, code: &str, rpc: &'static str) -> Result<Entry, Status> {
        let entry = self.registry.get(code).ok_or_else(|| {
            refusal(
                ErrorCode::UnknownCurrency,
                format!("currency {code} is not in the registry"),
            )
        })?;

        let now = Utc::now();
        if entry.is_stale(now) {
            tracing::warn!(
                event = "rate.stale",
                rpc,
                currency = code,
                ageHours = entry.age(now).map(|a| a.num_hours()).unwrap_or_default(),
                "serving a stale rate — the sync job has not landed"
            );
        }

        Ok(entry)
    }
}

#[tonic::async_trait]
impl CurrencyService for CurrencyApi {
    async fn convert_to_units(
        &self,
        request: Request<ConvertToUnitsRequest>,
    ) -> Result<Response<UnitsResponse>, Status> {
        let span = span("ConvertToUnits", &request);
        let _entered = span.enter();

        let request = request.get_ref();
        let entry = self.entry(&request.currency, "ConvertToUnits")?;

        // The rate plays NO part here: this is a change of representation of one and the same
        // amount, not an exchange. So the call works even before the first sync; otherwise a
        // client probing the scale at startup would never succeed against a fresh database.
        let units = money::to_units(request.amount, &entry.rate).map_err(arithmetic)?;

        Ok(Response::new(UnitsResponse { units }))
    }

    async fn convert_from_units(
        &self,
        request: Request<ConvertFromUnitsRequest>,
    ) -> Result<Response<AmountResponse>, Status> {
        let span = span("ConvertFromUnits", &request);
        let _entered = span.enter();

        let request = request.get_ref();
        let entry = self.entry(&request.currency, "ConvertFromUnits")?;

        let amount = money::from_units(request.units, &entry.rate).map_err(arithmetic)?;

        Ok(Response::new(AmountResponse { amount }))
    }

    async fn convert_units(
        &self,
        request: Request<ConvertUnitsRequest>,
    ) -> Result<Response<UnitsResponse>, Status> {
        let span = span("ConvertUnits", &request);
        let _entered = span.enter();

        let request = request.get_ref();

        let from = self.entry(&request.currency, "ConvertUnits")?;
        let to = self.entry(&request.to_currency, "ConvertUnits")?;

        // An exchange into itself is an identity and never reaches the rate checks: a
        // single-currency deployment may have no rate at all, yet it has nothing to convert either.
        if request
            .currency
            .trim()
            .eq_ignore_ascii_case(request.to_currency.trim())
        {
            return Ok(Response::new(UnitsResponse {
                units: request.units,
            }));
        }

        // A rate that NEVER arrived is not "one". Converting one-to-one would silently hand out
        // wrong money; this refusal makes it visible that the sync has not run.
        for (code, entry) in [(&request.currency, &from), (&request.to_currency, &to)] {
            if entry.updated_at.is_none() {
                return Err(refusal(
                    ErrorCode::RateMissing,
                    format!("currency {code} has never been quoted"),
                ));
            }
        }

        let units =
            money::convert_units(request.units, &from.rate, &to.rate).map_err(arithmetic)?;

        Ok(Response::new(UnitsResponse { units }))
    }
}

/// Call span carrying the trace id the caller attached. Without it a conversion shows up in the
/// log as a standalone event, unrelated to the upstream operation that triggered it.
fn span<T>(rpc: &'static str, request: &Request<T>) -> tracing::Span {
    let trace_id = request
        .metadata()
        .get(TRACE_HEADER)
        .and_then(|v| v.to_str().ok());

    call_span(rpc, trace_id)
}

/// A substantive refusal, with a stable code in the metadata.
///
/// The status code alone is not enough: "currency not in the registry" and "rate never arrived"
/// are both FAILED_PRECONDITION, yet different people fix them with different actions.
fn refusal(code: ErrorCode, message: String) -> Status {
    tracing::warn!(
        event = "convert.refused",
        code = code.as_str(),
        "{}",
        message
    );

    let mut status = match code {
        ErrorCode::InvalidAmount => Status::invalid_argument(message),
        ErrorCode::Internal => Status::internal(message),
        _ => Status::failed_precondition(message),
    };

    if let Ok(value) = code.as_str().parse() {
        status.metadata_mut().insert(ERROR_CODE_KEY, value);
    }

    status
}

fn arithmetic(error: RateError) -> Status {
    match error {
        RateError::NotFinite | RateError::OutOfRange => {
            refusal(ErrorCode::InvalidAmount, error.to_string())
        }
        RateError::BadRate => refusal(ErrorCode::RateMissing, error.to_string()),
    }
}
