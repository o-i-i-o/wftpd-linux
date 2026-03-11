#include "wftpg_bridge.h"
#include <QDebug>
#include <QByteArray>

WftpgBridge::WftpgBridge(QObject* parent)
    : QObject(parent)
    , m_state(nullptr)
    , m_ftpPort(21)
    , m_sftpPort(22)
    , m_ftpEnabled(true)
    , m_sftpEnabled(true)
{
    m_state = wftpg_init();
    if (m_state) {
        loadConfigCache();
    }
}

WftpgBridge::~WftpgBridge()
{
    if (m_state) {
        wftpg_free(m_state);
        m_state = nullptr;
    }
}

void WftpgBridge::loadConfigCache()
{
    if (!m_state) return;
    
    char* bindIp = nullptr;
    char* ftpHome = nullptr;
    char* sftpHome = nullptr;
    int ftpPort = 21, sftpPort = 22;
    int ftpEnabled = 1, sftpEnabled = 1;
    
    wftpg_get_config(m_state, &bindIp, &ftpPort, &sftpPort,
                     &ftpEnabled, &sftpEnabled, &ftpHome, &sftpHome);
    
    if (bindIp) {
        m_bindIp = QString::fromUtf8(bindIp);
        wftpg_free_string(bindIp);
    }
    if (ftpHome) {
        m_ftpHome = QString::fromUtf8(ftpHome);
        wftpg_free_string(ftpHome);
    }
    if (sftpHome) {
        m_sftpHome = QString::fromUtf8(sftpHome);
        wftpg_free_string(sftpHome);
    }
    
    m_ftpPort = ftpPort;
    m_sftpPort = sftpPort;
    m_ftpEnabled = ftpEnabled != 0;
    m_sftpEnabled = sftpEnabled != 0;
}

bool WftpgBridge::startFtp()
{
    if (!m_state) return false;
    int result = wftpg_start_ftp(m_state);
    if (result == 0) {
        emit ftpStatusChanged(true);
    }
    return result == 0;
}

void WftpgBridge::stopFtp()
{
    if (m_state) {
        wftpg_stop_ftp(m_state);
        emit ftpStatusChanged(false);
    }
}

bool WftpgBridge::startSftp()
{
    if (!m_state) return false;
    int result = wftpg_start_sftp(m_state);
    if (result == 0) {
        emit sftpStatusChanged(true);
    }
    return result == 0;
}

void WftpgBridge::stopSftp()
{
    if (m_state) {
        wftpg_stop_sftp(m_state);
        emit sftpStatusChanged(false);
    }
}

bool WftpgBridge::isFtpRunning()
{
    if (!m_state) return false;
    return wftpg_is_ftp_running(m_state) != 0;
}

bool WftpgBridge::isSftpRunning()
{
    if (!m_state) return false;
    return wftpg_is_sftp_running(m_state) != 0;
}

bool WftpgBridge::addUser(const QString& username, const QString& password,
                          const QString& homeDir, bool isAdmin)
{
    if (!m_state) return false;
    
    QByteArray userBytes = username.toUtf8();
    QByteArray passBytes = password.toUtf8();
    QByteArray homeBytes = homeDir.toUtf8();
    
    int result = wftpg_add_user(m_state, userBytes.constData(), passBytes.constData(),
                                 homeBytes.constData(), isAdmin ? 1 : 0);
    
    if (result == 0) {
        emit usersChanged();
    }
    return result == 0;
}

bool WftpgBridge::removeUser(const QString& username)
{
    if (!m_state) return false;
    
    QByteArray userBytes = username.toUtf8();
    int result = wftpg_remove_user(m_state, userBytes.constData());
    
    if (result == 0) {
        emit usersChanged();
    }
    return result == 0;
}

bool WftpgBridge::updateUserPassword(const QString& username, const QString& newPassword)
{
    if (!m_state) return false;
    
    QByteArray userBytes = username.toUtf8();
    QByteArray passBytes = newPassword.toUtf8();
    
    return wftpg_update_user_password(m_state, userBytes.constData(), passBytes.constData()) == 0;
}

int WftpgBridge::getUserCount()
{
    if (!m_state) return 0;
    return wftpg_get_user_count(m_state);
}

bool WftpgBridge::setUserEnabled(const QString& username, bool enabled)
{
    if (!m_state) return false;
    
    QByteArray userBytes = username.toUtf8();
    return wftpg_set_user_enabled(m_state, userBytes.constData(), enabled ? 1 : 0) == 0;
}

bool WftpgBridge::updateUserPermissions(const QString& username,
                                        bool canRead, bool canWrite, bool canDelete, bool canList,
                                        bool canMkdir, bool canRmdir, bool canRename, bool canAppend)
{
    if (!m_state) return false;
    
    QByteArray userBytes = username.toUtf8();
    return wftpg_update_user_permissions(m_state, userBytes.constData(),
                                          canRead ? 1 : 0, canWrite ? 1 : 0,
                                          canDelete ? 1 : 0, canList ? 1 : 0,
                                          canMkdir ? 1 : 0, canRmdir ? 1 : 0,
                                          canRename ? 1 : 0, canAppend ? 1 : 0) == 0;
}

QString WftpgBridge::getBindIp()
{
    return m_bindIp;
}

int WftpgBridge::getFtpPort()
{
    return m_ftpPort;
}

int WftpgBridge::getSftpPort()
{
    return m_sftpPort;
}

bool WftpgBridge::isFtpEnabled()
{
    return m_ftpEnabled;
}

bool WftpgBridge::isSftpEnabled()
{
    return m_sftpEnabled;
}

QString WftpgBridge::getFtpHome()
{
    return m_ftpHome;
}

QString WftpgBridge::getSftpHome()
{
    return m_sftpHome;
}

bool WftpgBridge::setConfig(const QString& bindIp, int ftpPort, int sftpPort,
                            bool ftpEnabled, bool sftpEnabled,
                            const QString& ftpHome, const QString& sftpHome)
{
    if (!m_state) return false;
    
    QByteArray bindBytes = bindIp.toUtf8();
    QByteArray ftpHomeBytes = ftpHome.toUtf8();
    QByteArray sftpHomeBytes = sftpHome.toUtf8();
    
    int result = wftpg_set_config(m_state, bindBytes.constData(),
                                   ftpPort, sftpPort,
                                   ftpEnabled ? 1 : 0, sftpEnabled ? 1 : 0,
                                   ftpHomeBytes.constData(), sftpHomeBytes.constData());
    
    if (result == 0) {
        m_bindIp = bindIp;
        m_ftpPort = ftpPort;
        m_sftpPort = sftpPort;
        m_ftpEnabled = ftpEnabled;
        m_sftpEnabled = sftpEnabled;
        m_ftpHome = ftpHome;
        m_sftpHome = sftpHome;
        emit configChanged();
    }
    
    return result == 0;
}

bool WftpgBridge::addAllowedIp(const QString& ip)
{
    if (!m_state) return false;
    
    QByteArray ipBytes = ip.toUtf8();
    return wftpg_add_allowed_ip(m_state, ipBytes.constData()) == 0;
}

bool WftpgBridge::removeAllowedIp(const QString& ip)
{
    if (!m_state) return false;
    
    QByteArray ipBytes = ip.toUtf8();
    return wftpg_remove_allowed_ip(m_state, ipBytes.constData()) == 0;
}

bool WftpgBridge::addDeniedIp(const QString& ip)
{
    if (!m_state) return false;
    
    QByteArray ipBytes = ip.toUtf8();
    return wftpg_add_denied_ip(m_state, ipBytes.constData()) == 0;
}

bool WftpgBridge::removeDeniedIp(const QString& ip)
{
    if (!m_state) return false;
    
    QByteArray ipBytes = ip.toUtf8();
    return wftpg_remove_denied_ip(m_state, ipBytes.constData()) == 0;
}

QStringList WftpgBridge::getAllowedIps()
{
    return QStringList();
}

QStringList WftpgBridge::getDeniedIps()
{
    return QStringList();
}

bool WftpgBridge::installService(const QString& binaryPath)
{
    if (!m_state) return false;
    
    QByteArray pathBytes = binaryPath.toUtf8();
    return wftpg_install_service(m_state, pathBytes.constData()) == 0;
}

bool WftpgBridge::uninstallService()
{
    if (!m_state) return false;
    return wftpg_uninstall_service(m_state) == 0;
}

bool WftpgBridge::startService()
{
    if (!m_state) return false;
    return wftpg_service_start(m_state) == 0;
}

bool WftpgBridge::stopService()
{
    if (!m_state) return false;
    return wftpg_service_stop(m_state) == 0;
}

bool WftpgBridge::enableService()
{
    if (!m_state) return false;
    return wftpg_service_enable(m_state) == 0;
}

bool WftpgBridge::disableService()
{
    if (!m_state) return false;
    return wftpg_service_disable(m_state) == 0;
}

bool WftpgBridge::isServiceRunning()
{
    if (!m_state) return false;
    return wftpg_is_service_running(m_state) != 0;
}

bool WftpgBridge::isServiceEnabled()
{
    if (!m_state) return false;
    return wftpg_is_service_enabled(m_state) != 0;
}

bool WftpgBridge::serviceExists()
{
    if (!m_state) return false;
    return wftpg_service_exists(m_state) != 0;
}

QList<WftpgUserData> WftpgBridge::getUserList()
{
    return QList<WftpgUserData>();
}

QList<WftpgLogEntry> WftpgBridge::getLogs(int count)
{
    return QList<WftpgLogEntry>();
}
