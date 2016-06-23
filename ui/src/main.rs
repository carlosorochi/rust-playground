#![feature(try_from)]
#![feature(custom_derive, plugin)]
#![plugin(serde_macros)]

#[macro_use]
extern crate log;
extern crate env_logger;
extern crate iron;
extern crate mount;
extern crate staticfile;
extern crate bodyparser;
extern crate serde;
extern crate serde_json;
extern crate mktemp;
#[macro_use]
extern crate quick_error;

use std::any::Any;
use std::convert::{TryFrom, TryInto};
use std::env;
use std::path::PathBuf;

use iron::prelude::*;
use iron::status;
use mount::Mount;
use serde::{Serialize, Deserialize};
use staticfile::Static;

use sandbox::Sandbox;

const DEFAULT_ADDRESS: &'static str = "127.0.0.1";
const DEFAULT_PORT: u16 = 5000;

mod sandbox;

fn main() {
    env_logger::init().expect("Unable to initialize logger");

    let root: PathBuf = env::var_os("PLAYGROUND_UI_ROOT").expect("Must specify PLAYGROUND_UI_ROOT").into();
    let address = env::var("PLAYGROUND_UI_ADDRESS").unwrap_or(DEFAULT_ADDRESS.to_string());
    let port = env::var("PLAYGROUND_UI_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);

    let mut mount = Mount::new();
    mount.mount("/", Static::new(&root));
    mount.mount("/compile", compile);
    mount.mount("/execute", execute);
    mount.mount("/format", format);

    info!("Starting the server on {}:{}", address, port);
    Iron::new(mount).http((&*address, port)).expect("Unable to start server");
}

fn compile(req: &mut Request) -> IronResult<Response> {
    with_sandbox(req, |sandbox, req: CompileRequest| {
        let req = try!(req.try_into());
        sandbox
            .compile(&req)
            .map(CompileResponse::from)
            .map_err(Error::Sandbox)
    })
}

fn execute(req: &mut Request) -> IronResult<Response> {
    with_sandbox(req, |sandbox, req: ExecuteRequest| {
        let req = try!(req.try_into());
        sandbox
            .execute(&req)
            .map(ExecuteResponse::from)
            .map_err(Error::Sandbox)
    })
}

fn format(req: &mut Request) -> IronResult<Response> {
    with_sandbox(req, |sandbox, req: FormatRequest| {
        sandbox
            .format(&req.into())
            .map(FormatResponse::from)
            .map_err(Error::Sandbox)
    })
}

fn with_sandbox<Req, Resp, F>(req: &mut Request, f: F) -> IronResult<Response>
    where F: FnOnce(Sandbox, Req) -> Result<Resp>,
          Req: Deserialize + Clone + Any + 'static,
          Resp: Serialize,
{
    let response = req.get::<bodyparser::Struct<Req>>()
        .map_err(Error::Deserialization)
        .and_then(|r| r.ok_or(Error::RequestMissing))
        .and_then(|req| {
            let sandbox = try!(Sandbox::new());
            let resp = try!(f(sandbox, req));
            let body = try!(serde_json::ser::to_string(&resp));
            Ok(body)
        });

    match response {
        Ok(body) => Ok(Response::with((status::Ok, body))),
        Err(err) => {
            let err = ErrorJson { error: err.to_string() };
            match serde_json::ser::to_string(&err) {
                Ok(error_str) => Ok(Response::with((status::InternalServerError, error_str))),
                Err(_) => Ok(Response::with((status::InternalServerError, FATAL_ERROR_JSON))),
            }
        },
    }
}

quick_error! {
    #[derive(Debug)]
    pub enum Error {
        Sandbox(err: sandbox::Error) {
            description("sandbox operation failed")
            display("Sandbox operation failed: {}", err)
            cause(err)
            from()
        }
        Serialization(err: serde_json::Error) {
            description("unable to serialize response")
            display("Unable to serialize response: {}", err)
            cause(err)
            from()
        }
        Deserialization(err: bodyparser::BodyError) {
            description("unable to deserialize request")
            display("Unable to deserialize request: {}", err)
            cause(err)
            from()
        }
        InvalidTarget(value: String) {
            description("an invalid target was passed")
            display("The value {:?} is not a valid target", value)
        }
        InvalidChannel(value: String) {
            description("an invalid channel was passed")
            display("The value {:?} is not a valid channel", value,)
        }
        InvalidMode(value: String) {
            description("an invalid mode was passed")
            display("The value {:?} is not a valid mode", value)
        }
        RequestMissing {
            description("no request was provided")
            display("No request was provided")
        }
    }
}

type Result<T> = ::std::result::Result<T, Error>;

const FATAL_ERROR_JSON: &'static str =
    r#"{"error": "Multiple cascading errors occurred, abandon all hope"}"#;

#[derive(Debug, Clone, Serialize)]
struct ErrorJson {
    error: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CompileRequest {
    target: String,
    channel: String,
    mode: String,
    tests: bool,
    code: String,
}

impl TryFrom<CompileRequest> for sandbox::CompileRequest {
    type Err = Error;

    fn try_from(me: CompileRequest) -> Result<Self> {
        Ok(sandbox::CompileRequest {
            target: try!(parse_target(&me.target)),
            channel: try!(parse_channel(&me.channel)),
            mode: try!(parse_mode(&me.mode)),
            tests: me.tests,
            code: me.code,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
struct CompileResponse {
    success: bool,
    code: String,
    stdout: String,
    stderr: String,
}

impl From<sandbox::CompileResponse> for CompileResponse {
    fn from(me: sandbox::CompileResponse) -> Self {
        CompileResponse {
            success: me.success,
            code: me.code,
            stdout: me.stdout,
            stderr: me.stderr,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ExecuteRequest {
    channel: String,
    mode: String,
    tests: bool,
    code: String,
}

impl TryFrom<ExecuteRequest> for sandbox::ExecuteRequest {
    type Err = Error;

    fn try_from(me: ExecuteRequest) -> Result<Self> {
        Ok(sandbox::ExecuteRequest {
            channel: try!(parse_channel(&me.channel)),
            mode: try!(parse_mode(&me.mode)),
            tests: me.tests,
            code: me.code,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
struct ExecuteResponse {
    success: bool,
    stdout: String,
    stderr: String,
}

impl From<sandbox::ExecuteResponse> for ExecuteResponse {
    fn from(me: sandbox::ExecuteResponse) -> Self {
        ExecuteResponse {
            success: me.success,
            stdout: me.stdout,
            stderr: me.stderr,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct FormatRequest {
    code: String,
}

impl From<FormatRequest> for sandbox::FormatRequest {
    fn from(me: FormatRequest) -> Self {
        sandbox::FormatRequest {
            code: me.code,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct FormatResponse {
    success: bool,
    code: String,
    stdout: String,
    stderr: String,
}

impl From<sandbox::FormatResponse> for FormatResponse {
    fn from(me: sandbox::FormatResponse) -> Self {
        FormatResponse {
            success: me.success,
            code: me.code,
            stdout: me.stdout,
            stderr: me.stderr,
        }
    }
}

fn parse_target(s: &str) -> Result<sandbox::CompileTarget> {
    Ok(match s {
        "asm" => sandbox::CompileTarget::Assembly,
        "llvm-ir" => sandbox::CompileTarget::LlvmIr,
        _ => return Err(Error::InvalidTarget(s.into()))
    })
}

fn parse_channel(s: &str) -> Result<sandbox::Channel> {
    Ok(match s {
        "stable" => sandbox::Channel::Stable,
        "beta" => sandbox::Channel::Beta,
        "nightly" => sandbox::Channel::Nightly,
        _ => return Err(Error::InvalidChannel(s.into()))
    })
}

fn parse_mode(s: &str) -> Result<sandbox::Mode> {
    Ok(match s {
        "debug" => sandbox::Mode::Debug,
        "release" => sandbox::Mode::Release,
        _ => return Err(Error::InvalidMode(s.into()))
    })
}
