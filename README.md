### env vars

following env vars have to be defined

```
APP__DB_PATH   // a directory where the app will store it's data
APP__ADMIN_ID  // bot's owner, who's apporving / rejecting users attempting to register
OPENAI_API_KEY // openai api key, you know :)
TELOXIDE_TOKEN // telegram bot token, refer to https://t.me/botfather
```

### build and deploy

I run it on my RaspberryPI, in case you want to build it for something else, please tweak `deploy`. 
