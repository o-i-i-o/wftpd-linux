#ifndef WFTPG_BRIDGE_H
#define WFTPG_BRIDGE_H

#include <QObject>
#include <QString>
#include <QStringList>
#include <QVariant>

extern "C" {
    void* wftpg_init();
    void wftpg_free(void* state);
    
    int wftpg_start_ftp(void* state);
    void wftpg_stop_ftp(void* state);
    int wftpg_start_sftp(void* state);
    void wftpg_stop_sftp(void* state);
    int wftpg_is_ftp_running(void* state);
    int wftpg_is_sftp_running(void* state);
    
    int wftpg_add_user(void* state, const char* username, const char* password, const char* home_dir, int is_admin);
    int wftpg_remove_user(void* state, const char* username);
    int wftpg_update_user_password(void* state, const char* username, const char* new_password);
    int wftpg_get_user_count(void* state);
    int wftpg_set_user_enabled(void* state, const char* username, int enabled);
    int wftpg_update_user_permissions(void* state, const char* username,
                                       int can_read, int can_write, int can_delete, int can_list,
                                       int can_mkdir, int can_rmdir, int can_rename, int can_append);
    
    void wftpg_get_config(void* state, char** bind_ip, int* ftp_port, int* sftp_port,
                          int* ftp_enabled, int* sftp_enabled, char** ftp_home, char** sftp_home);
    int wftpg_set_config(void* state, const char* bind_ip, int ftp_port, int sftp_port,
                         int ftp_enabled, int sftp_enabled, const char* ftp_home, const char* sftp_home);
    
    int wftpg_add_allowed_ip(void* state, const char* ip);
    int wftpg_remove_allowed_ip(void* state, const char* ip);
    int wftpg_add_denied_ip(void* state, const char* ip);
    int wftpg_remove_denied_ip(void* state, const char* ip);
    
    int wftpg_install_service(void* state, const char* binary_path);
    int wftpg_uninstall_service(void* state);
    int wftpg_service_start(void* state);
    int wftpg_service_stop(void* state);
    int wftpg_service_enable(void* state);
    int wftpg_service_disable(void* state);
    int wftpg_is_service_running(void* state);
    int wftpg_is_service_enabled(void* state);
    int wftpg_service_exists(void* state);
    
    void wftpg_free_string(char* s);
}

struct WftpgUserData {
    QString username;
    QString homeDir;
    bool enabled;
    bool isAdmin;
};

struct WftpgLogEntry {
    QString timestamp;
    QString level;
    QString source;
    QString message;
    QString clientIp;
};

class WftpgBridge : public QObject
{
    Q_OBJECT
    
public:
    explicit WftpgBridge(QObject* parent = nullptr);
    ~WftpgBridge();
    
    Q_INVOKABLE bool startFtp();
    Q_INVOKABLE void stopFtp();
    Q_INVOKABLE bool startSftp();
    Q_INVOKABLE void stopSftp();
    Q_INVOKABLE bool isFtpRunning();
    Q_INVOKABLE bool isSftpRunning();
    
    Q_INVOKABLE bool addUser(const QString& username, const QString& password, 
                              const QString& homeDir, bool isAdmin);
    Q_INVOKABLE bool removeUser(const QString& username);
    Q_INVOKABLE bool updateUserPassword(const QString& username, const QString& newPassword);
    Q_INVOKABLE int getUserCount();
    Q_INVOKABLE bool setUserEnabled(const QString& username, bool enabled);
    Q_INVOKABLE bool updateUserPermissions(const QString& username,
                                            bool canRead, bool canWrite, bool canDelete, bool canList,
                                            bool canMkdir, bool canRmdir, bool canRename, bool canAppend);
    
    Q_INVOKABLE QString getBindIp();
    Q_INVOKABLE int getFtpPort();
    Q_INVOKABLE int getSftpPort();
    Q_INVOKABLE bool isFtpEnabled();
    Q_INVOKABLE bool isSftpEnabled();
    Q_INVOKABLE QString getFtpHome();
    Q_INVOKABLE QString getSftpHome();
    
    Q_INVOKABLE bool setConfig(const QString& bindIp, int ftpPort, int sftpPort,
                               bool ftpEnabled, bool sftpEnabled,
                               const QString& ftpHome, const QString& sftpHome);
    
    Q_INVOKABLE bool addAllowedIp(const QString& ip);
    Q_INVOKABLE bool removeAllowedIp(const QString& ip);
    Q_INVOKABLE bool addDeniedIp(const QString& ip);
    Q_INVOKABLE bool removeDeniedIp(const QString& ip);
    Q_INVOKABLE QStringList getAllowedIps();
    Q_INVOKABLE QStringList getDeniedIps();
    
    Q_INVOKABLE bool installService(const QString& binaryPath);
    Q_INVOKABLE bool uninstallService();
    Q_INVOKABLE bool startService();
    Q_INVOKABLE bool stopService();
    Q_INVOKABLE bool enableService();
    Q_INVOKABLE bool disableService();
    Q_INVOKABLE bool isServiceRunning();
    Q_INVOKABLE bool isServiceEnabled();
    Q_INVOKABLE bool serviceExists();
    
    Q_INVOKABLE QList<WftpgUserData> getUserList();
    Q_INVOKABLE QList<WftpgLogEntry> getLogs(int count);
    
signals:
    void ftpStatusChanged(bool running);
    void sftpStatusChanged(bool running);
    void usersChanged();
    void configChanged();
    void logUpdated(const WftpgLogEntry& entry);
    
private:
    void* m_state;
    void loadConfigCache();
    
    QString m_bindIp;
    int m_ftpPort;
    int m_sftpPort;
    bool m_ftpEnabled;
    bool m_sftpEnabled;
    QString m_ftpHome;
    QString m_sftpHome;
};

#endif
