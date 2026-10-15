//! Public catalog queries do not open a local inference session or daemon.

use std::time::Duration;

use omni_core::choose::{self, Pick};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    CliError, Output, Result, no_args, output, print_value, reject_options, take_providers,
    take_value,
};

#[derive(Deserialize)]
struct Category {
    id: String,
    says: String,
}

#[derive(Deserialize)]
struct Categories {
    categories: Vec<Category>,
}

#[derive(Deserialize)]
struct Models {
    models: Vec<Pick>,
}

#[derive(Deserialize)]
struct ShortlistPick {
    #[serde(flatten)]
    pick: Pick,
    available: bool,
}

#[derive(Deserialize)]
struct Best {
    pick: Pick,
    shortlist: Vec<ShortlistPick>,
}

pub fn categories(mut args: Vec<String>) -> Result<()> {
    let format = output(&mut args, false)?;
    no_args("categories", args)?;
    let value = request(&format!("{}/categories", choose::backend()), &[])?;
    let response: Categories = decode(&value)?;
    if format == Output::Json {
        print_value(&value);
    } else {
        println!("category\tdescription");
        for category in response.categories {
            println!("{}\t{}", category.id, category.says);
        }
    }
    Ok(())
}

pub fn models(mut args: Vec<String>) -> Result<()> {
    let format = output(&mut args, false)?;
    let provider = take_value(&mut args, "--provider")?;
    no_args("models", args)?;
    if provider.as_ref().is_some_and(|value| value.trim().is_empty()) {
        return Err(CliError("--provider needs a provider name".into()));
    }
    let query: Vec<_> = provider.into_iter().map(|p| ("provider", p)).collect();
    let value = request(&format!("{}/models", choose::backend()), &query)?;
    let response: Models = decode(&value)?;
    if format == Output::Json {
        print_value(&value);
    } else {
        println!("provider\tmodel\teffort\tfast");
        for pick in response.models {
            println!(
                "{}\t{}\t{}\t{}",
                pick.provider, pick.model, pick.effort, pick.fast
            );
        }
    }
    Ok(())
}

pub fn best(mut args: Vec<String>) -> Result<()> {
    let format = output(&mut args, false)?;
    let providers = take_providers(&mut args)?;
    reject_options(&args)?;
    if args.len() != 1 || args[0].trim().is_empty() {
        return Err(CliError(
            "best needs exactly one category; try `omni categories`".into(),
        ));
    }
    let mut query = vec![("key", args[0].trim().to_lowercase())];
    if let Some(providers) = providers {
        if providers.is_empty() {
            return Err(CliError("--providers needs at least one provider".into()));
        }
        query.push(("providers", providers.join("+")));
    }
    let value = request(&choose::remote(), &query)?;
    let response: Best = decode(&value)?;
    if format == Output::Json {
        print_value(&value);
    } else {
        let pick = response.pick;
        println!(
            "best\t{}\t{}\t{}\tfast={}",
            pick.provider, pick.model, pick.effort, pick.fast
        );
        println!("rank\tprovider\tmodel\teffort\tfast\teligible");
        for (index, row) in response.shortlist.iter().enumerate() {
            let pick = &row.pick;
            println!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                index + 1,
                pick.provider,
                pick.model,
                pick.effort,
                pick.fast,
                row.available
            );
        }
    }
    Ok(())
}

fn decode<'a, T: Deserialize<'a>>(value: &'a Value) -> Result<T> {
    T::deserialize(value).map_err(|err| {
        CliError(format!(
            "backend returned an invalid catalog response: {err}"
        ))
    })
}

fn request(url: &str, query: &[(&str, String)]) -> Result<Value> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .build()
        .new_agent();
    let mut request = agent.get(url).header("accept", "application/json");
    for (name, value) in query {
        request = request.query(name, value);
    }
    let mut response = request
        .call()
        .map_err(|err| CliError(format!("cannot reach catalog backend: {err}")))?;
    let status = response.status();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|err| CliError(format!("cannot read catalog response: {err}")))?;
    let value = serde_json::from_str::<Value>(&body);
    if !status.is_success() {
        let detail = value
            .as_ref()
            .ok()
            .and_then(|v| v.get("error"))
            .and_then(Value::as_str);
        return Err(CliError(match detail {
            Some(detail) => format!("catalog backend returned HTTP {status}: {detail}"),
            None => format!("catalog backend returned HTTP {status}"),
        }));
    }
    value.map_err(|err| CliError(format!("backend returned invalid JSON: {err}")))
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;

    use serde_json::json;

    use super::*;

    fn server(status: &str, body: &str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap() == 0 || header == "\r\n" {
                    break;
                }
            }
            stream.write_all(response.as_bytes()).unwrap();
            line
        });
        (format!("http://{address}"), handle)
    }

    #[test]
    fn queries_encode_provider_lists_and_preserve_the_response() {
        let body = json!({"pick": {"provider": "codex-app-server", "model": "gpt-example"}, "shortlist": []});
        let (url, server) = server("200 OK", &body.to_string());
        let response = request(
            &format!("{url}/choose.json"),
            &[
                ("key", "code & design".into()),
                ("providers", "claude-code-cli+codex-app-server".into()),
            ],
        )
        .unwrap();
        assert_eq!(response, body);
        let line = server.join().unwrap();
        assert!(line.contains("key=code%20%26%20design"), "{line}");
        assert!(
            line.contains("providers=claude-code-cli%2Bcodex-app-server"),
            "{line}"
        );
        let best: Best = decode(&response).unwrap();
        assert_eq!(best.pick.model, "gpt-example");
    }

    #[test]
    fn server_errors_keep_the_actionable_message() {
        let (url, server) = server("400 Bad Request", r#"{"error":"no such key"}"#);
        let error = request(&url, &[]).unwrap_err().to_string();
        assert!(error.contains("HTTP 400"), "{error}");
        assert!(error.contains("no such key"), "{error}");
        server.join().unwrap();
    }

    #[test]
    fn malformed_success_is_an_error_instead_of_an_empty_catalog() {
        let (url, server) = server("200 OK", "<html>not a catalog</html>");
        assert!(
            request(&url, &[])
                .unwrap_err()
                .to_string()
                .contains("invalid JSON")
        );
        server.join().unwrap();
        assert!(decode::<Models>(&json!({"models": [{"provider": "test"}]})).is_err());
        assert!(decode::<Categories>(&json!({})).is_err());
        assert!(decode::<Best>(&json!({"pick": null, "shortlist": []})).is_err());
    }

    #[test]
    fn invalid_options_fail_before_a_network_request() {
        assert!(
            best(vec![])
                .unwrap_err()
                .to_string()
                .contains("one category")
        );
        assert!(
            best(vec!["code".into(), "--providers".into(), ",".into()])
                .unwrap_err()
                .to_string()
                .contains("at least one provider")
        );
        assert!(
            models(vec!["--provider".into()])
                .unwrap_err()
                .to_string()
                .contains("needs a value")
        );
        assert!(categories(vec!["--frames".into()]).is_err());
    }
}
