use super::super::match_kind::FileType;
use super::analyze;

#[test]
fn detects_core_signature() {
    // Shape from redux/src/utils/actionTypes.ts after bundling/minification.
    let src = r#"var ActionTypes={INIT:"@@redux/INIT"+r(),REPLACE:"@@redux/REPLACE"+r(),PROBE_UNKNOWN_ACTION:function(){return"@@redux/PROBE_UNKNOWN_ACTION"+r()}};function e(n){return"Minified Redux error #"+n+"; visit https://redux.js.org/Errors?code="+n}"#;
    let m = analyze(src, Some("t.js"), FileType::Js);
    assert!(m.iter().any(|x| x.match_kind == "redux_core"));
    // Both @@redux/* tokens and the error URL should be found.
    assert!(m.iter().filter(|x| x.match_kind == "redux_core").count() >= 4);
}

#[test]
fn detects_slice_thunk_and_marker() {
    // Minified webpack interop form: (0,n.createSlice)({...}).
    let src = r#"var s=(0,n.createSlice)({ name:"counter",initialState:{ value:0 },reducers:{ increment:function(e){ e.value++ } } });var f=(0,r.createAsyncThunk)("users/fetchById",function(e){return e});var k=Symbol.for("rtk-slice-createasyncthunk");"#;
    let m = analyze(src, Some("t.js"), FileType::Js);
    assert!(
        m.iter()
            .any(|x| x.match_kind == "rtk_slice" && x.match_value == "counter")
    );
    assert!(
        m.iter()
            .any(|x| x.match_kind == "rtk_async_thunk" && x.match_value == "users/fetchById")
    );
    assert!(
        m.iter()
            .any(|x| x.match_kind == "rtk_marker" && x.match_value == "rtk-slice-createasyncthunk")
    );
}

#[test]
fn detects_rtk_query_api_and_endpoints() {
    let src = r#"var api=(0,t.createApi)({reducerPath:"api",baseQuery:(0,t.fetchBaseQuery)({baseUrl:"https://api.example.com/v1/"}),tagTypes:["Post","User"],endpoints:function(build){return{getPosts:build.query({query:function(){return"posts"}}),addPost:build.mutation({query:function(e){return{url:"posts",method:"POST",body:e}}})}}});"#;
    let m = analyze(src, Some("t.js"), FileType::Js);

    let api = m
        .iter()
        .find(|x| x.match_kind == "rtk_query_api")
        .expect("rtk_query_api match");
    assert_eq!(api.match_value, "https://api.example.com/v1/");
    let meta = api.metadata.as_ref().expect("metadata");
    assert_eq!(meta["reducer_path"], "api");
    assert_eq!(meta["tags"][0], "Post");
    assert_eq!(meta["tags"][1], "User");

    assert!(
        m.iter()
            .any(|x| x.match_kind == "rtk_query_endpoint" && x.match_value == "getPosts")
    );
    assert!(
        m.iter()
            .any(|x| x.match_kind == "rtk_query_endpoint" && x.match_value == "addPost")
    );
}

#[test]
fn detects_minified_endpoints_with_mangled_builder() {
    // Production-minified: the `endpoints:(e)=>` builder is a single char,
    // so a hardcoded `build.` would miss these entirely.
    let src = r#"var api=(0,t.createApi)({reducerPath:"a",baseQuery:(0,t.fetchBaseQuery)({baseUrl:"https://x.dev/"}),endpoints:(e)=>({getX:e.query({query:()=>"x"}),setY:e.mutation({query:()=>({url:"y",method:"POST"})})})});"#;
    let m = analyze(src, Some("t.js"), FileType::Js);
    assert!(
        m.iter()
            .any(|x| x.match_kind == "rtk_query_endpoint" && x.match_value == "getX"),
        "minified query endpoint missed: {m:?}"
    );
    assert!(
        m.iter()
            .any(|x| x.match_kind == "rtk_query_endpoint" && x.match_value == "setY"),
        "minified mutation endpoint missed: {m:?}"
    );
}

#[test]
fn rejects_non_rtk_query_method_calls() {
    // No `endpoints:` builder in scope, so `db.query(` / `gql.mutation(`
    // must NOT be reported as RTK Query endpoints.
    let src = r#"var rows={fetchAll:db.query("SELECT 1"),apply:gql.mutation("x")};"#;
    let m = analyze(src, Some("t.js"), FileType::Js);
    assert!(
        !m.iter().any(|x| x.match_kind == "rtk_query_endpoint"),
        "false-positive endpoint on plain method calls: {m:?}"
    );
}

#[test]
fn endpoints_non_callback_value_does_not_whitelist() {
    // `endpoints: someRef` is a bare reference, not a builder callback, so a
    // later `someRef.query(` must not be promoted to an RTK endpoint.
    let src = r#"var x={endpoints: someRef, other: someRef.query("z")};"#;
    let m = analyze(src, Some("t.js"), FileType::Js);
    assert!(
        !m.iter().any(|x| x.match_kind == "rtk_query_endpoint"),
        "non-callback endpoints value wrongly whitelisted a builder: {m:?}"
    );
}

#[test]
fn drops_ambiguous_metadata_for_multiple_createapi() {
    // Two createApi blocks: a global first-match reducerPath would be wrong
    // for one of them, so the metadata is dropped rather than mis-attributed.
    let src = r#"var a=(0,t.createApi)({reducerPath:"authApi",baseQuery:(0,t.fetchBaseQuery)({baseUrl:"https://auth.dev/"}),endpoints:(b)=>({login:b.mutation({})})});var u=(0,t.createApi)({reducerPath:"userApi",baseQuery:(0,t.fetchBaseQuery)({baseUrl:"https://user.dev/"}),endpoints:(b)=>({me:b.query({})})});"#;
    let m = analyze(src, Some("t.js"), FileType::Js);
    let ep = m
        .iter()
        .find(|x| x.match_kind == "rtk_query_endpoint")
        .expect("endpoint detected");
    assert!(
        ep.metadata.as_ref().unwrap()["reducer_path"].is_null(),
        "ambiguous reducer_path must not be attributed: {ep:?}"
    );
}
