//! Redux / Redux Toolkit (RTK) / RTK Query recon analyzer.
//!
//! Detects Redux-family state-management signatures that survive minification in
//! production bundles; emits one [`AnalyzerMatch`] per hit. Each detector maps to
//! a distinct `snake_case` `match_kind` prefixed with the framework.
//!
//! Signatures confirmed against the cloned current source:
//! - `@@redux/INIT|REPLACE|PROBE_UNKNOWN_ACTION` -> redux/src/utils/actionTypes.ts
//! - `https://redux.js.org/Errors?code=` -> redux/src/utils/formatProdErrorMessage.ts
//! - `Symbol.for("rtk-slice-createasyncthunk")` -> RTK `createSlice.ts`
//! - `Symbol.for("rtk-state-proxy-original")` -> RTK `combineSlices.ts`
//! - `Symbol.for("RTKQ/skipToken")` -> RTK Query selectors
//! - `createSlice({name,reducers})` + action type prefixes -> RTK `createSlice.ts`
//! - `createAsyncThunk("prefix")` + lifecycle suffixes -> RTK `createAsyncThunk.ts`
//! - `fetchBaseQuery({baseUrl})`, `reducerPath`, `tagTypes`, `build.query` -> RTK Query

mod detector;
mod patterns;
#[cfg(test)]
mod tests;

pub use detector::analyze;
