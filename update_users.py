#!/usr/bin/env python3
import json

users_data = {
    "users": {
        "123": {
            "username": "123",
            "password_hash": "$argon2id$v=19$m=19456,t=2,p=1$zVU9bYRdS+dwFDQ5i6uEWA$P3uZUEgD016Njm53UxJljsDcekEK+/bTE8FLFbe/V8Q",
            "home_dir": "/home/wftpg/123",
            "permissions": {
                "can_read": True,
                "can_write": True,
                "can_delete": True,
                "can_list": True,
                "can_mkdir": True,
                "can_rmdir": True,
                "can_rename": True,
                "can_append": True,
                "quota_mb": 1,
                "speed_limit_kbps": None
            },
            "created_at": "2026-03-14T06:50:12.347486227Z",
            "last_login": None,
            "enabled": True,
            "is_admin": False
        }
    }
}

print(json.dumps(users_data))
