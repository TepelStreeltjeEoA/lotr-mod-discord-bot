use poise::serenity::utils::Colour;

use crate::api;
use crate::database;
use crate::utils;
use crate::{Context, Result};

fn parse_motd<T: ToString>(motd: T) -> String {
    let motd = motd.to_string();
    let mut res = String::with_capacity(motd.len());
    let mut stack: Vec<&str> = Vec::new();
    let mut is_token = false;
    for c in motd.chars() {
        if c == '§' {
            is_token = true;
        } else if is_token {
            is_token = false;
            match c {
                '0'..='9' | 'a'..='f' | 'k' | 'r' => {
                    if !stack.is_empty() {
                        stack.drain(..).rev().for_each(|s| res.push_str(s));
                        res.push('\u{200B}');
                    }
                }
                'l' => {
                    stack.push("**");
                    res.push_str("**");
                }

                'n' => {
                    stack.push("__");
                    res.push_str("__");
                }
                'm' => {
                    stack.push("~~");
                    res.push_str("~~");
                }
                'o' => {
                    stack.push("*");
                    res.push('*');
                }
                _ => {
                    res.push('§');
                    res.push(c)
                }
            }
        } else {
            res.push(c);
        }
    }
    stack.drain(..).rev().for_each(|t| res.push_str(t));
    res
}

/// Display the server status and a list of online players
#[poise::command(slash_command, category = "Minecraft Server Commands")]
pub async fn online(
    ctx: Context<'_>,
    #[description = "An ip to query"] ip: Option<String>,
) -> Result {
    let ip = if let Some(ip) = ip {
        ip
    } else {
        match database::minecraft::get_minecraft_ip(&ctx).await {
            Ok(ip) => ip,
            Err(e) => {
                utils::error_printer!(e.as_ref());
                ctx.defer_ephemeral().await?;
                ctx.say("There is no registered IP on this server. Set one using  `!ip set <server ip>`.")
            .await?;
                return Ok(());
            }
        }
    };

    match api::minecraft::get_server_status(&ctx, &ip).await {
        Ok(server) => {
            if server.online {
                ctx.send(|m| {
                    m.embed(|e| {
                        e.colour(Colour::DARK_GREEN);
                        e.thumbnail(format!("https://eu.mc-api.net/v3/server/favicon/{}", &ip));
                        e.title("Server online!");
                        let desc = if let Some(motd) = &server.motd.map(|d| d.raw.join("\n")) {
                            format!("{}\n\n", parse_motd(motd))
                        } else {
                            "".into()
                        };
                        e.description(format!("{}**IP:**  `{}`", desc, &ip,));
                        if let Some(players) = &server.players {
                            e.field(
                                format!("Players: {}/{}", players.online, players.max),
                                players
                                    .list
                                    .as_ref()
                                    .map(|s| {
                                        let res = s.join(", ").replace("_", "\\_");
                                        if res.len() > 1024 {
                                            "Too many usernames to display!".into()
                                        } else {
                                            res
                                        }
                                    })
                                    .unwrap_or_else(|| "[]()".into()),
                                false,
                            );
                        }
                        e
                    });
                    m
                })
                .await?;
            } else {
                ctx.send(|m| {
                    m.embed(|e| {
                        e.colour(Colour::RED);
                        e.title("Server offline...");
                        e.description(format!("**IP:**  `{}`", &ip));
                        e
                    });
                    m
                })
                .await?;
            }
        }
        Err(e) => {
            ctx.defer_ephemeral().await?;
            ctx.send(|m| {
                m.embed(|e| {
                    e.colour(Colour::RED)
                        .title("Could not get server status...")
                        .description(format!("IP `{}` looks unreachable.", ip))
                })
            })
            .await?;
            utils::error_printer!(e.as_ref())
        }
    }
    Ok(())
}

/// Display the server's ip address
#[poise::command(slash_command, category = "Minecraft Server Commands")]
pub async fn ip(_ctx: Context<'_>) -> Result {
    Ok(())
}

/// Display the server's ip address
#[poise::command(slash_command, category = "Minecraft Server Commands")]
pub async fn display(ctx: Context<'_>) -> Result {
    match database::minecraft::get_minecraft_ip(&ctx).await {
        Ok(ip) => {
            ctx.send(|m| {
                m.embed(|e| {
                    e.colour(Colour::TEAL);
                    e.title("Server IP:");
                    e.description(format!("`{}`", ip));
                    e
                })
            })
            .await?;
        }
        Err(e) => {
            utils::error_printer!(e.as_ref());
            ctx.defer_ephemeral().await?;
            ctx.say(
                "There is no registered IP on this server. Set one using  `!ip set <server ip>`.",
            )
            .await?;
        }
    }
    Ok(())
}

/// Set the server's IP address
#[poise::command(
    slash_command,
    ephemeral,
    check = "crate::checks::is_admin",
    category = "Minecraft Server Commands"
)]
pub async fn set(ctx: Context<'_>, #[description = "The IP address to set"] ip: String) -> Result {
    if let Err(e) = database::minecraft::set_minecraft_ip(&ctx, &ip).await {
        utils::error_printer!(e.as_ref());
        ctx.say("Oops, the bot failed to set the IP address of the server...")
            .await?;
    } else {
        ctx.say(format!(
            "Successfully set the Minecraft server IP to `{}`",
            ip
        ))
        .await?;

        let guild_id = ctx.guild_id().ok_or("Not in a guild")?;

        guild_id
            .create_application_command(ctx.discord(), |b| {
                *b = ctx
                    .framework()
                    .options()
                    .commands
                    .iter()
                    .find(|c| c.name == "online")
                    .map(|c| c.create_as_slash_command())
                    .flatten()
                    .expect("No /online command found!");
                b
            })
            .await?;
    }
    Ok(())
}

/// Delete the server's IP address
#[poise::command(
    slash_command,
    ephemeral,
    check = "crate::checks::is_admin",
    category = "Minecraft Server Commands"
)]
pub async fn delete(ctx: Context<'_>) -> Result {
    if let Ok(ip) = database::minecraft::get_minecraft_ip(&ctx).await {
        database::minecraft::delete_minecraft_ip(&ctx).await?;
        ctx.say(format!(
            "Successfully removed ip  `{}`  from this server",
            ip
        ))
        .await?;
    } else {
        ctx.say("No registered Minecraft IP for this server.")
            .await?;
    }

    let guild_id = ctx.guild_id().ok_or("Not in a guild")?;

    if let Some(command) = guild_id
        .get_application_commands(ctx.discord())
        .await?
        .iter()
        .find(|&c| c.name == "online")
    {
        guild_id
            .delete_application_command(ctx.discord(), command.id)
            .await?;
    }
    Ok(())
}
