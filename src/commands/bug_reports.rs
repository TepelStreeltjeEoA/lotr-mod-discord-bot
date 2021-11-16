use serenity::client::Context;
use serenity::collector::CollectComponentInteraction;
use serenity::framework::standard::{macros::command, Args, CommandResult};
use serenity::model::{
    interactions::message_component::{ButtonStyle, MessageComponentInteraction},
    prelude::*,
};
use serenity::prelude::*;
use std::time::Duration;

use crate::check::*;
use crate::constants::{LOTR_DISCORD, MANAGE_BOT_PERMS, OWNER_ID};
use crate::database::admin_data::is_admin_function;
use crate::database::bug_reports::{
    add_bug_report, add_link, change_bug_status, change_title, get_bug_from_id, get_bug_list,
    get_bug_statistics, remove_link, switch_edition, BugOrder, BugStatus,
};
use crate::failure;

pub const TERMITE_EMOJI: EmojiId = EmojiId(839479605467152384);

macro_rules! termite {
    ($ctx:ident, $msg:ident) => {{
        $msg.react(
            $ctx,
            ReactionType::from(EmojiIdentifier {
                animated: false,
                id: TERMITE_EMOJI,
                name: "bug".into(),
            }),
        )
        .await?;
    }};
}

macro_rules! termite_success {
    ($ctx:ident, $msg:ident) => {
        termite!($ctx, $msg);
    };
    ($ctx:ident, $msg:ident, $single_message:expr) => {{
        $msg.reply($ctx, $single_message).await?;
        termite!($ctx, $msg);
    }};
    ($ctx:ident, $msg:ident, $($success:tt)*) => {{
        $msg.reply($ctx, format!($($success)*)).await?;
        termite!($ctx, $msg);
    }};
}

#[command]
#[checks(is_admin, is_lotr_discord)]
#[aliases(report)]
pub async fn track(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let legacy = args.current().map(|s| s == "legacy").unwrap_or_default();
    if legacy {
        args.advance();
    }
    let status = args.single::<BugStatus>().unwrap_or_default();

    let title = args.rest();
    if title.is_empty() {
        failure!(ctx, msg, "You must provide a title for the bug report!");
        return Ok(());
    }

    let referenced_message = if let Some(message) = &msg.referenced_message {
        message
    } else {
        failure!(ctx, msg, "You must reference a message in your bug report!");
        return Ok(());
    };

    match add_bug_report(ctx, referenced_message, title.to_string(), status, legacy).await {
        Ok(bug_id) => termite_success!(
            ctx,
            msg,
            "Tracking bug LOTR-{} (priority: `{}`)",
            bug_id,
            status
        ),
        Err(e) => {
            failure!(ctx, msg, "Could not submit the bug report!");
            return Err(e);
        }
    }

    Ok(())
}

enum Either<'a> {
    Message(&'a Message),
    Interaction(&'a MessageComponentInteraction),
}

impl<'a> Either<'a> {
    async fn failure(&self, ctx: &Context, message: &str) -> Result<(), SerenityError> {
        match self {
            Either::Message(msg) => failure!(ctx, msg, message),
            Either::Interaction(interaction) => {
                interaction
                    .create_interaction_response(ctx, |r| {
                        r.kind(InteractionResponseType::ChannelMessageWithSource)
                            .interaction_response_data(|d| {
                                d.flags(InteractionApplicationCommandCallbackDataFlags::EPHEMERAL)
                                    .content(message)
                            })
                    })
                    .await?;
            }
        }
        Ok(())
    }
}

async fn display_bugs(
    ctx: &Context,
    status: Option<BugStatus>,
    limit: u32,
    display_order: BugOrder,
    legacy: Option<bool>,
    page: u32,
    reply_to: Either<'_>,
) -> Result<Option<Message>, SerenityError> {
    assert_ne!(page, 0);

    if let Some((bugs, total_bugs)) =
        get_bug_list(ctx, status, limit, display_order, legacy, page - 1).await
    {
        if ((page - 1) * limit) >= total_bugs {
            reply_to.failure(ctx, "Page number too high, consider calling `!bugs` and using the navigation arrows.").await?;
            return Err(SerenityError::Other("page_too_high"));
        }

        let title;
        let content_alt;
        let content;
        let colour;
        if let Some(status) = status {
            title = format!(
                "{} Bug reports (Status: {}){} (Total: {})",
                status.marker(),
                status,
                if let Some(b) = legacy {
                    if b {
                        " [legacy]"
                    } else {
                        " [renewed]"
                    }
                } else {
                    ""
                },
                total_bugs
            );
            content_alt = "_No open bugs!_";
            content = bugs
                .iter()
                .map(|b| {
                    format!(
                        "{}{}",
                        b,
                        if legacy.is_none() && b.legacy {
                            " [legacy]"
                        } else {
                            ""
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            colour = status.colour();
        } else {
            title = format!(
                "Open bug reports{} (Total: {})",
                if let Some(b) = legacy {
                    if b {
                        " [legacy]"
                    } else {
                        " [renewed]"
                    }
                } else {
                    ""
                },
                total_bugs
            );
            content_alt = "_No bugs with this status!_";
            content = bugs
                .iter()
                .map(|b| {
                    format!(
                        "{} {}{}",
                        b.status.marker(),
                        b,
                        if legacy.is_none() && b.legacy {
                            " [legacy]"
                        } else {
                            ""
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            colour = serenity::utils::Colour::LIGHT_GREY;
        }

        if content.len() > 4096 {
            reply_to
                .failure(
                    ctx,
                    "Too many bugs to display. Consider lowering the limit.",
                )
                .await?;
            return Err(SerenityError::Other("too_many_bugs"));
        }

        macro_rules! create_embed_reponse {
            () => {
                |e| {
                    e.author(|a| {
                        a.name("LOTR Mod Bugtracker");
                        a.icon_url(crate::constants::TERMITE_IMAGE);
                        a
                    });
                    e.colour(colour);
                    e.title(title);
                    e.description(if bugs.is_empty() {
                        content_alt
                    } else {
                        &content
                    });
                    e.footer(|f| {
                        f.text(format!(
                            "Page {}/{}",
                            page,
                            (total_bugs.max(1) - 1) / limit + 1
                        ))
                    });
                    e
                }
            };
        }

        macro_rules! create_buttons {
            () => {
                |c| {
                    c.create_action_row(|a| {
                        a.create_button(|b| {
                            b.style(ButtonStyle::Secondary);
                            b.label("Previous");
                            b.custom_id("previous_page");
                            b.emoji(ReactionType::Unicode("⬅️".into()));
                            if page <= 1 {
                                b.disabled(true);
                            }
                            b
                        });
                        a.create_button(|b| {
                            b.style(ButtonStyle::Secondary);
                            b.label("Next");
                            b.custom_id("next_page");
                            b.emoji(ReactionType::Unicode("➡️".into()));
                            if (page * limit) >= total_bugs {
                                b.disabled(true);
                            }
                            b
                        });
                        a
                    });
                    c
                }
            };
        }

        match reply_to {
            Either::Interaction(interaction) => {
                interaction
                    .create_interaction_response(ctx, |r| {
                        r.kind(InteractionResponseType::UpdateMessage)
                            .interaction_response_data(|m| {
                                m.embeds([])
                                    .create_embed(create_embed_reponse!())
                                    .components(create_buttons!())
                            })
                    })
                    .await?;

                Ok(None)
            }
            Either::Message(msg) => {
                let response_message = msg
                    .channel_id
                    .send_message(ctx, |m| {
                        m.embed(create_embed_reponse!())
                            .components(create_buttons!())
                    })
                    .await?;
                Ok(Some(response_message))
            }
        }
    } else {
        Err(SerenityError::Other(
            "Could not get bugs from the database!",
        ))
    }
}

#[command]
#[aliases(bugs)]
#[sub_commands(bugtracker_help)]
pub async fn buglist(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let legacy = args
        .current()
        .map(|s| match s {
            "legacy" => Some(true),
            "renewed" => Some(false),
            _ => None,
        })
        .flatten();
    if legacy.is_some() {
        args.advance();
    }
    let status = args.single::<BugStatus>().ok();

    let mut display_order = match args.current() {
        Some("latest") => BugOrder::Chronological(false),
        Some("oldest") => BugOrder::Chronological(true),
        Some("highest") => BugOrder::Priority(false),
        Some("lowest") => BugOrder::Priority(true),
        _ => BugOrder::None,
    };
    if let BugOrder::None = display_order {
        display_order = BugOrder::Chronological(false);
    } else {
        args.advance();
    }

    let mut page = args.single::<u32>().unwrap_or(1).max(1);

    let limit = if args.current() == Some("limit") {
        args.advance();
        args.single::<u32>().ok()
    } else {
        None
    }
    .unwrap_or(10);

    let mut response_message = match display_bugs(
        ctx,
        status,
        limit,
        display_order,
        legacy,
        page,
        Either::Message(msg),
    )
    .await
    {
        Ok(Some(msg)) => msg,
        Ok(None) => unreachable!(),
        Err(SerenityError::Other("page_too_high" | "too_many_bugs")) => return Ok(()),
        Err(e) => return Err(e.into()),
    };

    while let Some(interaction) = CollectComponentInteraction::new(ctx)
        .timeout(Duration::from_secs(60))
        .channel_id(msg.channel_id)
        .message_id(response_message.id)
        .await
    {
        if interaction.user.id != msg.author.id {
            interaction.create_interaction_response(ctx, |r| {
                r.kind(InteractionResponseType::ChannelMessageWithSource);
                r.interaction_response_data(|d| {
                    d.content("You are not the original user of the command! Call `!bugs` yourself to use the buttons.");
                    d.flags(InteractionApplicationCommandCallbackDataFlags::EPHEMERAL)
                })
            })
            .await?;
            continue;
        }
        match interaction.data.custom_id.as_str() {
            "previous_page" => {
                if page != 0 {
                    page -= 1;
                }
            }
            "next_page" => {
                page += 1;
            }
            _ => (),
        }

        let _ = display_bugs(
            ctx,
            status,
            limit,
            display_order,
            legacy,
            page,
            Either::Interaction(interaction.as_ref()),
        )
        .await?;
    }

    response_message.edit(ctx, |m| m.components(|c| c)).await?;

    Ok(())
}

#[command]
#[checks(is_lotr_discord)]
#[sub_commands(
    track,
    bug_status,
    resolve,
    bug_close,
    bug_link,
    bug_rename,
    stats,
    bug_toggle_edition,
    bugtracker_help
)]
pub async fn bug(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let bug_id = if let Ok(bug_id) = args.single::<String>() {
        if let Ok(bug_id) = bug_id
            .to_uppercase()
            .trim_start_matches("LOTR-")
            .parse::<u64>()
        {
            bug_id
        } else {
            failure!(ctx, msg, "`{}` is not a valid bug id!", bug_id);
            return Ok(());
        }
    } else {
        failure!(ctx, msg, "The first argument must be a bug id.");
        return Ok(());
    };

    let mut bug = match get_bug_from_id(ctx, bug_id).await {
        Ok(bug) => bug,
        Err(e) => {
            failure!(ctx, msg, "Bug LOTR-{} does not exist!", bug_id);
            return Err(e);
        }
    };

    macro_rules! create_bug_embed {
        ($bug:expr, $linked_message:expr) => {
            |e| {
                e.author(|a| {
                    a.name("LOTR Mod Bugtracker");
                    a.icon_url(crate::constants::TERMITE_IMAGE);
                    a
                });
                e.colour($bug.status.colour());
                e.title(format!(
                    "{} LOTR-{}: {}{}",
                    $bug.status.marker(),
                    $bug.bug_id,
                    $bug.title,
                    if $bug.legacy { " [legacy]" } else { "" }
                ));
                if let Ok(ref message) = $linked_message {
                    e.description(&message.content);
                    if let Some(image) = message.attachments.get(0) {
                        e.image(&image.url);
                    }
                    e.footer(|f| {
                        f.text(format!(
                            "Status: {} • Submitted by {}",
                            $bug.status, &message.author.name
                        ))
                    });
                } else {
                    e.footer(|f| f.text(format!("Status: {}", $bug.status)));
                }
                if !$bug.links.is_empty() {
                    e.field(
                        "Additional information",
                        &$bug
                            .links
                            .iter()
                            .map(|link| link.to_string())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        false,
                    );
                }
                e.timestamp(&$bug.timestamp);
                e
            }
        };
    }

    macro_rules! create_bug_buttons {
        ($message_link:expr) => {
            |c| {
                if let Some(ref link) = $message_link {
                    c.create_action_row(|a| {
                        a.create_button(|b| {
                            b.style(ButtonStyle::Link).label("Message link").url(link)
                        })
                    });
                }
                c
            }
        };
        ($message_link:expr, $create_buttons:expr) => {
            |c| {
                if $message_link.is_some() || $create_buttons {
                    c.create_action_row(|a| {
                        if let Some(link) = $message_link.as_ref() {
                            a.create_button(|b| {
                                b.style(ButtonStyle::Link).label("Message link").url(link)
                            });
                        }
                        if $create_buttons {
                            a.create_button(|b| {
                                b.style(ButtonStyle::Success)
                                    .label("Resolve")
                                    .custom_id("resolve_bug")
                            });

                            a.create_button(|b| {
                                b.style(ButtonStyle::Danger)
                                    .label("Close")
                                    .custom_id("close_bug")
                            });
                        }
                        a
                    });
                }
                c
            }
        };
    }

    let linked_message = bug
        .channel_id
        .message(ctx, bug.message_id)
        .await
        .map(|mut m| {
            m.guild_id = Some(LOTR_DISCORD);
            m
        });
    let message_link = linked_message.as_ref().map(|m| m.link()).ok();

    let is_lotr_discord = msg.guild_id == Some(LOTR_DISCORD);
    let is_admin = if let Some(guild_id) = msg.guild_id {
        is_admin_function(ctx, guild_id, msg.author.id)
            .await
            .unwrap_or_default()
            || crate::utils::has_permission(ctx, guild_id, msg.author.id, MANAGE_BOT_PERMS).await
    } else {
        false
    };

    let mut create_buttons = bug.status != BugStatus::Resolved
        && bug.status != BugStatus::Closed
        && (msg.author.id == OWNER_ID || (is_lotr_discord && is_admin));

    let mut response_message = msg
        .channel_id
        .send_message(ctx, |m| {
            m.embed(create_bug_embed!(bug, linked_message))
                .components(create_bug_buttons!(message_link, create_buttons))
        })
        .await?;

    if create_buttons {
        // Listen to interactions for 120 seconds
        while let Some(interaction) = CollectComponentInteraction::new(ctx)
            .timeout(Duration::from_secs(60))
            .channel_id(msg.channel_id)
            .message_id(response_message.id)
            .await
        {
            if interaction.user.id != msg.author.id {
                interaction
                    .create_interaction_response(ctx, |r| {
                        r.kind(InteractionResponseType::ChannelMessageWithSource)
                            .interaction_response_data(|d| {
                                d.flags(InteractionApplicationCommandCallbackDataFlags::EPHEMERAL)
                                    .content("You are not allowed to modify bug status!")
                            })
                    })
                    .await?;
            } else {
                let new_status = match interaction.data.custom_id.as_str() {
                    "resolve_bug" => BugStatus::Resolved,
                    "close_bug" => BugStatus::Closed,
                    _ => unreachable!(),
                };

                change_bug_status(ctx, bug_id, new_status).await?;

                bug.status = new_status;

                interaction
                    .create_interaction_response(ctx, |r| {
                        r.kind(InteractionResponseType::UpdateMessage)
                            .interaction_response_data(|m| {
                                m.embeds([])
                                    .create_embed(create_bug_embed!(bug, linked_message))
                                    .components(create_bug_buttons!(message_link))
                            })
                    })
                    .await?;

                create_buttons = false;

                break;
            }
        }

        if create_buttons {
            // If no interaction was received after timeout, remove the buttons
            response_message
                .edit(ctx, |m| m.components(create_bug_buttons!(message_link)))
                .await?;
        }
    }

    Ok(())
}

#[command]
#[checks(is_lotr_discord, is_admin)]
#[aliases("status")]
pub async fn bug_status(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    if let Ok(bug_id) = args.single::<String>() {
        if let Ok(bug_id) = bug_id
            .to_uppercase()
            .trim_start_matches("LOTR-")
            .parse::<u64>()
        {
            if let Ok(new_status) = args.single::<BugStatus>() {
                match change_bug_status(ctx, bug_id, new_status).await {
                    Ok(old_status) => termite_success!(
                        ctx,
                        msg,
                        "Status changed for LOTR-{} from `{}` to `{}`!",
                        bug_id,
                        old_status,
                        new_status
                    ),
                    Err(e) => {
                        failure!(ctx, msg, "The bug LOTR-{} does not exist!", bug_id);
                        return Err(e);
                    }
                }
            } else {
                failure!(ctx, msg, "The second argument must be a bug status.")
            }
        } else {
            failure!(ctx, msg, "`{}` is not a valid bug id!", bug_id)
        }
    } else {
        failure!(ctx, msg, "The first argument must be a bug id.")
    }
    Ok(())
}

#[command]
#[checks(is_lotr_discord, is_admin)]
pub async fn resolve(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    if let Ok(bug_id) = args.single::<String>() {
        if let Ok(bug_id) = bug_id
            .to_uppercase()
            .trim_start_matches("LOTR-")
            .parse::<u64>()
        {
            if let Err(e) = change_bug_status(ctx, bug_id, BugStatus::Resolved).await {
                failure!(ctx, msg, "The bug LOTR-{} does not exist!", bug_id);
                return Err(e);
            } else {
                termite_success!(ctx, msg, "LOTR-{} has been marked as resolved.", bug_id)
            }
        } else {
            failure!(ctx, msg, "`{}` is not a valid bug id!", bug_id)
        }
    } else {
        failure!(ctx, msg, "The first argument must be a bug id.")
    }
    Ok(())
}

#[command]
#[checks(is_lotr_discord, is_admin)]
#[aliases("close")]
pub async fn bug_close(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    if let Ok(bug_id) = args.single::<String>() {
        if let Ok(bug_id) = bug_id
            .to_uppercase()
            .trim_start_matches("LOTR-")
            .parse::<u64>()
        {
            if let Err(e) = change_bug_status(ctx, bug_id, BugStatus::Closed).await {
                failure!(ctx, msg, "The bug LOTR-{} does not exist!", bug_id);
                return Err(e);
            } else {
                termite_success!(ctx, msg, "LOTR-{} has been marked as closed.", bug_id)
            }
        } else {
            failure!(ctx, msg, "`{}` is not a valid bug id!", bug_id)
        }
    } else {
        failure!(ctx, msg, "The first argument must be a bug id.")
    }
    Ok(())
}

#[command]
#[checks(is_lotr_discord, is_admin)]
#[sub_commands(bug_link_remove)]
#[aliases("link")]
pub async fn bug_link(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    if let Ok(bug_id) = args.single::<String>() {
        if let Ok(bug_id) = bug_id
            .to_uppercase()
            .trim_start_matches("LOTR-")
            .parse::<u64>()
        {
            if let Some(message) = &msg.referenced_message {
                let title = args.rest();
                if title.is_empty() {
                    failure!(ctx, msg, "Specify a title for your message link!");
                    return Ok(());
                }
                if let Some(link_id) = add_link(ctx, bug_id, &message.link(), title).await {
                    termite_success!(ctx, msg, "Added link #{} to LOTR-{}", link_id, bug_id);
                } else {
                    failure!(ctx, msg, "LOTR-{} does not exist!", bug_id);
                }
            } else if let Ok(link) = args.single::<String>() {
                let title = args.rest();
                if title.is_empty() {
                    failure!(ctx, msg, "Specify a title for your message link!");
                    return Ok(());
                }
                if let Some(link_id) = add_link(ctx, bug_id, &link, title).await {
                    termite_success!(ctx, msg, "Added link #{} to LOTR-{}", link_id, bug_id);
                } else {
                    failure!(ctx, msg, "LOTR-{} does not exist!", bug_id);
                }
            } else {
                failure!(ctx, msg, "You need to either reference a message or specify a link to add to the bug report.");
            }
        } else {
            failure!(ctx, msg, "`{}` is not a valid bug id!", bug_id);
        }
    } else {
        failure!(ctx, msg, "The first argument must be a bug id.");
    }
    Ok(())
}

#[command]
#[checks(is_admin, is_lotr_discord)]
#[aliases("remove")]
pub async fn bug_link_remove(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    if let Ok(bug_id) = args.single::<String>() {
        if let Ok(bug_id) = bug_id
            .to_uppercase()
            .trim_start_matches("LOTR-")
            .parse::<u64>()
        {
            let link_id = args.single::<String>();
            if let Ok(link_id) = link_id {
                if let Ok(link_id) = link_id.trim_start_matches('#').parse::<u64>() {
                    if remove_link(ctx, bug_id, link_id).await.is_ok() {
                        termite_success!(
                            ctx,
                            msg,
                            "Successfully removed link #{} from LOTR-{}",
                            link_id,
                            bug_id
                        );
                    } else {
                        failure!(
                            ctx,
                            msg,
                            "Link #{} does not exist in LOTR-{}",
                            link_id,
                            bug_id
                        );
                    }
                } else {
                    failure!(ctx, msg, "`{}` is not a valid link id!", link_id);
                }
            } else {
                failure!(ctx, msg, "The second argument must be a valid link id.");
            }
        } else {
            failure!(ctx, msg, "`{}` is not a valid bug id!", bug_id);
        }
    } else {
        failure!(ctx, msg, "The first argument must be a bug id.");
    }
    Ok(())
}

#[command]
#[checks(is_lotr_discord, is_admin)]
#[aliases("toggle")]
pub async fn bug_toggle_edition(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    if let Ok(bug_id) = args.single::<String>() {
        if let Ok(bug_id) = bug_id
            .to_uppercase()
            .trim_start_matches("LOTR-")
            .parse::<u64>()
        {
            if let Some(legacy) = switch_edition(ctx, bug_id).await {
                termite_success!(
                    ctx,
                    msg,
                    "LOTR-{} has been changed from {}",
                    bug_id,
                    if legacy {
                        "renewed to legacy"
                    } else {
                        "legacy to renewed"
                    }
                );
            } else {
                failure!(ctx, msg, "The bug LOTR-{} does not exist!", bug_id);
            }
        } else {
            failure!(ctx, msg, "`{}` is not a valid bug id!", bug_id);
        }
    } else {
        failure!(ctx, msg, "The first argument must be a bug id.");
    }
    Ok(())
}

#[command]
#[checks(is_lotr_discord, is_admin)]
#[aliases(rename)]
pub async fn bug_rename(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    if let Ok(bug_id) = args.single::<String>() {
        if let Ok(bug_id) = bug_id
            .to_uppercase()
            .trim_start_matches("LOTR-")
            .parse::<u64>()
        {
            let new_title = args.rest();
            if new_title.is_empty() {
                failure!(ctx, msg, "You must specify a new title for LOTR-{}", bug_id);
            } else if change_title(ctx, bug_id, new_title).await.is_ok() {
                termite_success!(
                    ctx,
                    msg,
                    "Successfully changed the title of LOTR-{}",
                    bug_id
                );
            } else {
                failure!(ctx, msg, "LOTR-{} does not exist!", bug_id);
            }
        } else {
            failure!(ctx, msg, "`{}` is not a valid bug id!", bug_id);
        }
    } else {
        failure!(ctx, msg, "The first argument must be a bug id.");
    }
    Ok(())
}

#[command]
#[checks(is_lotr_discord)]
#[aliases(statistics)]
pub async fn stats(ctx: &Context, msg: &Message) -> CommandResult {
    if let Some([resolved, low, medium, high, critical, closed, forgevanilla, total, legacy]) =
        get_bug_statistics(ctx).await
    {
        msg.channel_id
            .send_message(ctx, |m| {
                m.embed(|e| {
                    e.author(|a| {
                        a.name("LOTR Mod Bugtracker");
                        a.icon_url(crate::constants::TERMITE_IMAGE);
                        a
                    });
                    e.colour(serenity::utils::Colour::TEAL);
                    e.field(
                        "Bugtracker statistics",
                        format!(
                            "{} resolved
{} closed
{} forge or vanilla

_Open bugs: {}_
{} with low priority
{} with medium priority
{} with high priority
{} critical bugs

**Total: {} tracked bugs**
\t_including {} legacy bugs_
",
                            resolved,
                            closed,
                            forgevanilla,
                            total - resolved - closed - forgevanilla,
                            low,
                            medium,
                            high,
                            critical,
                            total,
                            legacy,
                        ),
                        false,
                    );
                    e
                })
            })
            .await?;
    } else {
        failure!(ctx, msg, "Could not fetch bugtracker statistics");
    }
    Ok(())
}

#[command]
#[checks(is_admin, is_lotr_discord)]
#[aliases("help")]
pub async fn bugtracker_help(ctx: &Context, msg: &Message) -> CommandResult {
    crate::commands::help::display_bugtracker_help(ctx, msg).await
}
