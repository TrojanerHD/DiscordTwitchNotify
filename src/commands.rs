use poise::serenity_prelude::{Channel, GuildId};

use crate::{Action, ContextData, GuildConfig, GuildsConfig};

use crate::store;
type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, ContextData, Error>;

#[poise::command(
    slash_command,
    subcommands("add", "remove", "list"),
    default_member_permissions = "MANAGE_GUILD"
)]
pub async fn streamer(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Add a streamer to get notified
#[poise::command(slash_command, guild_only, ephemeral)]
async fn add(
    ctx: Context<'_>,
    #[description = "Streamer to add"] streamer: String,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().expect("Could not read guild id");
    let mut cache = ctx.data().cache.lock().await;
    let guild_config = guild_config(&mut cache.guilds, guild_id);
    if guild_config
        .streamers
        .iter()
        .any(|x| x.to_lowercase() == streamer.to_lowercase())
    {
        drop(cache);
        ctx.reply(format!("Streamer {} already added", streamer.clone()))
            .await?;
        return Ok(());
    }
    guild_config.streamers.push(streamer.clone());
    if store::store_config(cache.guilds.clone()).is_err() {
        drop(cache);
        eprintln!("Could not store streamer {streamer}");
        ctx.reply("An error occured while trying to store the streamer!")
            .await?;
        // TODO: Remove streamer again
        // guild_config
        //     .streamers
        //     .remove(guild_config.streamers.len() - 1);
        return Ok(());
    }
    drop(cache);

    ctx.data()
        .streamer_tx
        .send_async((streamer, Action::Add))
        .await?;

    ctx.reply("Streamer added").await?;
    Ok(())
}

/// Remove a streamer from the notification list
#[poise::command(slash_command, guild_only, ephemeral)]
async fn remove(
    ctx: Context<'_>,
    #[description = "Streamer to remove"] streamer: String,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().expect("Could not read guild id");
    let mut cache = ctx.data().cache.lock().await;
    let guild_config = guild_config(&mut cache.guilds, guild_id);
    let Some(idx_to_remove) = guild_config
        .streamers
        .iter()
        .position(|x| x.to_lowercase() == streamer.to_lowercase())
    else {
        drop(cache);
        ctx.reply(format!("Streamer {} does not exist", streamer.clone()))
            .await?;
        return Ok(());
    };
    guild_config.streamers.remove(idx_to_remove);
    if store::store_config(cache.guilds.clone()).is_err() {
        drop(cache);
        eprintln!("Could not remove streamer {streamer}");
        ctx.reply("An error occured while trying to store the streamer!")
            .await?;
        // TODO: Add streamer again
        // guild_config.streamers.push(streamer);
        return Ok(());
    }
    drop(cache);

    ctx.data().streamer_tx.send((streamer, Action::Remove))?;
    ctx.reply("Streamer removed").await?;
    Ok(())
}

/// List all added streamers
#[poise::command(slash_command, guild_only, ephemeral)]
async fn list(ctx: Context<'_>) -> Result<(), Error> {
    let mut cache = ctx.data().cache.lock().await;
    let guild_id = ctx.guild_id().expect("Could not read guild id");
    let guild_config = guild_config(&mut cache.guilds, guild_id);
    let streamers = guild_config.streamers.clone();
    drop(cache);
    if streamers.clone().is_empty() {
        ctx.reply("No streamers added").await?;
        return Ok(());
    };
    ctx.reply(format!("Streamers: {}", streamers.join(", ")))
        .await?;
    Ok(())
}

fn guild_config(guilds: &mut GuildsConfig, guild_id: GuildId) -> &mut GuildConfig {
    guilds.entry(guild_id).or_insert(GuildConfig {
        streamers: Vec::new(),
        channel: None,
    })
}

/// Specify which channel the bot should send notifications to
#[poise::command(
    slash_command,
    subcommands("set", "reset"),
    default_member_permissions = "MANAGE_GUILD"
)]
pub async fn channel(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Set a channel in which stream notifications will be sent
#[poise::command(slash_command, guild_only, ephemeral)]
pub async fn set(
    ctx: Context<'_>,
    #[description = "Which channel to send live notifications to"] channel: Channel,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().expect("Could not read guild id");
    let mut cache = ctx.data().cache.lock().await;
    let guild_config = guild_config(&mut cache.guilds, guild_id);
    guild_config.channel = Some(channel.id());

    if store::store_config(cache.guilds.clone()).is_err() {
        drop(cache);
        eprintln!("Could not set channel to {}", channel.id());
        ctx.reply("An error occured while trying to store the channel setting!")
            .await?;
        // TODO: Unset channel again
        // guild_config.channel = None;
        return Ok(());
    }
    drop(cache);
    ctx.reply("Channel successfully set").await?;
    Ok(())
}

/// Disable notifications in this guild
#[poise::command(slash_command, guild_only, ephemeral)]
pub async fn reset(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().expect("Could not read guild id");
    let mut cache = ctx.data().cache.lock().await;
    let guild_config = guild_config(&mut cache.guilds, guild_id);
    guild_config.channel = None;
    if store::store_config(cache.guilds.clone()).is_err() {
        drop(cache);
        eprintln!("Could not unset channel");
        ctx.reply("An error occured while trying to store the channel setting!")
            .await?;
        // TODO: Set channel again
        return Ok(());
    }
    drop(cache);
    ctx.reply("Channel unset").await?;
    Ok(())
}
