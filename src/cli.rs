// Copyright 2019 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use log::info;
use prettytable::Table;

use routing::Action;
use safe_auth::authd;
use safe_auth::{
    acc_info, authed_apps, authorise_app, create_acc, log_in, revoke_app, AuthedAppsList,
};
use safe_authenticator::Authenticator;
use safe_core::ipc::req::IpcReq;
use structopt::StructOpt;
extern crate serde;
extern crate serde_json;

use std::fs;
use std::io::{stdin, stdout, Write};

use serde::Deserialize;
extern crate envy;

#[derive(Deserialize, Debug)]
struct Environment {
    safe_auth_secret: Option<String>,
    safe_auth_password: Option<String>,
}

#[derive(Debug)]
struct LoginDetails {
    secret: String,
    password: String,
}

#[derive(StructOpt, Debug)]
/// Manage SAFE Network authorisations and accounts.
pub struct CmdArgs {
    /// A config file to read secret/password from. This is a temporary convenience function, which is not recommended. (Storing login information unencrypted is not secure.)
    #[structopt(short = "c", long = "config")]
    config_file_str: Option<String>,
    /// The encoded authorisation request string
    #[structopt(short = "r", long = "req")]
    req_str: Option<String>,
    /// The invitation token for creating a new SAFE Network account
    #[structopt(short = "i", long = "invite-token")]
    invite: Option<String>,
    /// Get account's balance
    #[structopt(short = "b", long = "balance")]
    balance: bool,
    /// Get list of authorised apps
    #[structopt(short = "a", long = "apps")]
    apps: bool,
    /// The application's ID to revoke all authorised permissions from
    #[structopt(short = "k", long = "revoke")]
    app_id: Option<String>,
    /// Pretty print
    #[structopt(short = "y", long = "pretty")]
    pretty: bool,
    /// Port number where the Authenticator webservice shall be listening to
    #[structopt(short = "d", long = "daemon")]
    port: Option<u16>,
}

pub fn run() -> Result<(), String> {
    // Let's first get all the arguments passed in
    let args = CmdArgs::from_args();

    let login_details = get_login_details(&args)?;

    let authenticator: Authenticator;
    // If an invite token is provided, create a SAFE account, otherwise
    // just login. In both cases we use the instantiated authenticator
    // for all subsequent operations, even for the daemon services.
    if let Some(invite) = &args.invite {
        authenticator = create_acc(&invite, &login_details.secret, &login_details.password)?;
        if args.pretty {
            println!("Account was created successfully!");
        }
    } else {
        authenticator = log_in(&login_details.secret, &login_details.password)?;
        if args.pretty {
            println!("Logged in the SAFE Network successfully!");
        }
    }

    // Authorise the application if a auth req string was provided
    if let Some(req) = &args.req_str {
        let auth_response = authorise_app(&authenticator, &req, prompt_to_allow)?;
        if args.pretty {
            print!("Authorisation response string: ");
        }
        println!("{}", auth_response);
    }

    // Show the account's current balance if requested
    if args.balance {
        let (mutations_done, mutations_available) = acc_info(&authenticator)?;
        if args.pretty {
            print!("Account's current balance (PUTs done/available): ");
        }
        println!("{}/{}", mutations_done, mutations_available);
    };

    // Handle revoke arg if provided
    if let Some(app_id) = &args.app_id {
        revoke_app(&authenticator, app_id.clone())?;
        if args.pretty {
            println!("Authorised permissions were revoked for app '{}'", app_id);
        }
    }

    // List authorised apps if requested
    if args.apps {
        let authed_apps = authed_apps(&authenticator)?;
        if args.pretty {
            pretty_print_authed_apps(authed_apps);
        } else {
            parsable_list_authed_apps(authed_apps);
        }
    };

    if let Some(host_port) = args.port {
        authd::run(host_port, Some(authenticator));
    }

    Ok(())
}

// Private helper functions

fn get_login_details(args: &CmdArgs) -> Result<LoginDetails, String> {
    let mut the_secret: String = String::from("");
    let mut the_password: String = String::from("");

    let environment_details = envy::from_env::<Environment>().unwrap();

    if let Some(safe_auth_secret) = environment_details.safe_auth_secret {
        the_secret = safe_auth_secret;
        info!("Using secret from provided ENV var: safe_auth_secret")
    }

    if let Some(safe_auth_password) = environment_details.safe_auth_password {
        the_password = safe_auth_password;
        info!("Using password from provided ENV var: safe_auth_password")
    }

    if the_secret.is_empty() ^ the_password.is_empty() {
        return Err("Both the secret and password environment variables must be set to be used for SAFE login.".to_string());
    }

    if the_secret.is_empty() || the_password.is_empty() {
        if let Some(config_file_str) = &args.config_file_str {
            let file = match fs::File::open(&config_file_str) {
                Ok(file) => file,
                Err(error) => {
                    return Err(format!("Error reading config file. {}", error.to_string()));
                }
            };

            let json: serde_json::Value = serde_json::from_reader(file).unwrap();

            eprintln!("Warning! Storing your secret/password in plaintext in a config file is not secure." );

            if let Some(secret) = json.get("secret") {
                the_secret = secret.to_string();
            } else {
                return Err("The config files's secret field cannot be empty".to_string());
            }

            if let Some(password) = json.get("password") {
                the_password = password.to_string();
            } else {
                return Err("The config files's password field cannot be empty".to_string());
            }
        } else {
            // Prompt the user for the SAFE account credentials
            the_secret = rpassword::read_password_from_tty(Some("Secret: ")).unwrap();
            the_password = rpassword::read_password_from_tty(Some("Password: ")).unwrap();
        }
    }

    if the_secret.is_empty() || the_password.is_empty() {
        return Err(String::from(
            "Neither the secret nor password can be empty.",
        ));
    }

    let details = LoginDetails {
        secret: the_secret,
        password: the_password,
    };

    Ok(details)
}

fn pretty_print_authed_apps(authed_apps: Vec<AuthedAppsList>) {
    let mut table = Table::new();
    table.add_row(row!["Authorised Applications"]);
    table.add_row(row!["Id", "Name", "Vendor", "Permissions"]);

    let all_app_iterator = authed_apps.iter();
    for app_info in all_app_iterator {
        let mut row = String::from("");
        for (cont, perms) in app_info.perms.iter() {
            row += &format!("{}: {:?}\n", cont, perms);
        }
        table.add_row(row![
            app_info.app.id,
            app_info.app.name,
            // app_info.app.scope || "",
            app_info.app.vendor,
            row,
        ]);
    }
    table.printstd();
}

fn parsable_list_authed_apps(authed_apps: Vec<AuthedAppsList>) {
    println!("APP ID\tNAME\tVENDOR\tPERMISSIONS");
    let all_app_iterator = authed_apps.iter();
    for app_info in all_app_iterator {
        let mut row = format!(
            "{}\t{:?}\t{:?}\t[",
            &app_info.app.id, &app_info.app.name, &app_info.app.vendor
        );
        let mut it = app_info.perms.iter();
        while let Some((cont, perms)) = it.next() {
            row = row + &format!("{:?}:", cont);
            let mut it2 = perms.iter();
            while let Some(perm) = it2.next() {
                row = row + &format!("{:?}", perm);
                if it2.size_hint().0 > 0 {
                    row += "|";
                };
            }
            if it.size_hint().0 > 0 {
                row += ",";
            };
        }
        println!("{}]", row)
    }
}

fn prompt_to_allow(req: IpcReq) -> bool {
    match req {
        IpcReq::Auth(app_auth_req) => {
            println!("The following application authorisation request was received:");
            let mut table = Table::new();
            table.add_row(row!["Id", "Name", "Vendor", "Permissions requested"]);
            table.add_row(row![
                app_auth_req.app.id,
                app_auth_req.app.name,
                // app_auth_req.app.scope || "",
                app_auth_req.app.vendor,
                format!(
                    "Own container: {}\nDefault containers: {:?}",
                    app_auth_req.app_container, app_auth_req.containers
                ),
            ]);
            table.printstd();
        }
        IpcReq::Containers(cont_req) => {
            println!("The following authorisation request for containers was received:");
            println!("{:?}", cont_req);
            let mut table = Table::new();
            table.add_row(row!["Id", "Name", "Vendor", "Permissions requested"]);
            table.add_row(row![
                cont_req.app.id,
                cont_req.app.name,
                // cont_req.app.scope || "",
                cont_req.app.vendor,
                format!("{:?}", cont_req.containers)
            ]);
            table.printstd();
        }
        IpcReq::ShareMData(share_mdata_req) => {
            println!("The following authorisation request to share a MutableData was received:");
            let mut row = String::from("");
            for mdata in share_mdata_req.mdata.iter() {
                row += &format!(
                    "Type tag: {}\nXoR name: {}",
                    mdata.type_tag,
                    mdata.name.to_hex()
                );
                let insert_perm = if mdata.perms.is_allowed(Action::Insert).unwrap_or(false) {
                    " Insert"
                } else {
                    ""
                };
                let update_perm = if mdata.perms.is_allowed(Action::Update).unwrap_or(false) {
                    " Update"
                } else {
                    ""
                };
                let delete_perm = if mdata.perms.is_allowed(Action::Delete).unwrap_or(false) {
                    " Delete"
                } else {
                    ""
                };
                let manage_perm = if mdata
                    .perms
                    .is_allowed(Action::ManagePermissions)
                    .unwrap_or(false)
                {
                    " ManagePermissions"
                } else {
                    ""
                };
                row += &format!(
                    "\nPermissions:{}{}{}{}\n\n",
                    insert_perm, update_perm, delete_perm, manage_perm
                );
            }
            let mut table = Table::new();
            table.add_row(row![
                "Id",
                "Name",
                "Vendor",
                "MutableData's requested to share"
            ]);
            table.add_row(row![
                share_mdata_req.app.id,
                share_mdata_req.app.name,
                // share_mdata_req.app.scope || "",
                share_mdata_req.app.vendor,
                row
            ]);
            table.printstd();
        }
        IpcReq::Unregistered(_) => {
            // we simply allow unregistered authorisation requests
            return true;
        }
    };

    let mut prompt = String::new();
    print!("Allow authorisation? [y/N]: ");
    let _ = stdout().flush();
    stdin()
        .read_line(&mut prompt)
        .expect("Did not enter a correct string. Authorisation will be denied.");
    if let Some('\n') = prompt.chars().next_back() {
        prompt.pop();
    }
    if let Some('\r') = prompt.chars().next_back() {
        prompt.pop();
    }

    if prompt.to_lowercase() == "y" {
        println!("Authorisation will be allowed");
        true
    } else {
        println!("Authorisation will be denied");
        false
    }
}
