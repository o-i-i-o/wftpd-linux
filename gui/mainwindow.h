#ifndef MAINWINDOW_H
#define MAINWINDOW_H

#include <QMainWindow>
#include <QTabWidget>
#include <QPushButton>
#include <QLabel>
#include <QLineEdit>
#include <QSpinBox>
#include <QCheckBox>
#include <QTableWidget>
#include <QTextEdit>
#include <QListWidget>
#include <QGroupBox>
#include <QTimer>
#include "wftpg_bridge.h"

class MainWindow : public QMainWindow
{
    Q_OBJECT

public:
    explicit MainWindow(QWidget* parent = nullptr);
    ~MainWindow();

private slots:
    void onFtpStartClicked();
    void onFtpStopClicked();
    void onSftpStartClicked();
    void onSftpStopClicked();
    void onConfigSaveClicked();
    void onUserAddClicked();
    void onUserRemoveClicked();
    void onUserEditClicked();
    void onIpAddAllowedClicked();
    void onIpRemoveAllowedClicked();
    void onIpAddDeniedClicked();
    void onIpRemoveDeniedClicked();
    void onServiceInstallClicked();
    void onServiceUninstallClicked();
    void onServiceStartClicked();
    void onServiceStopClicked();
    void onServiceEnableClicked();
    void onServiceDisableClicked();
    void onRefreshLogs();
    void onFtpStatusChanged(bool running);
    void onSftpStatusChanged(bool running);
    void onBrowseFtpHome();
    void onBrowseSftpHome();

private:
    void setupUi();
    void setupServerTab();
    void setupUsersTab();
    void setupSecurityTab();
    void setupServiceTab();
    void setupLogsTab();
    void loadConfig();
    void loadUsers();
    void updateServerStatus();
    void updateServiceStatus();
    void addLogEntry(const QString& timestamp, const QString& level,
                     const QString& source, const QString& message, const QString& clientIp);

    WftpgBridge* m_bridge;
    
    QTabWidget* m_tabWidget;
    
    QWidget* m_serverTab;
    QLineEdit* m_bindIpEdit;
    QSpinBox* m_ftpPortSpin;
    QSpinBox* m_sftpPortSpin;
    QCheckBox* m_ftpEnabledCheck;
    QCheckBox* m_sftpEnabledCheck;
    QLineEdit* m_ftpHomeEdit;
    QLineEdit* m_sftpHomeEdit;
    QPushButton* m_ftpStartBtn;
    QPushButton* m_ftpStopBtn;
    QPushButton* m_sftpStartBtn;
    QPushButton* m_sftpStopBtn;
    QLabel* m_ftpStatusLabel;
    QLabel* m_sftpStatusLabel;
    
    QWidget* m_usersTab;
    QTableWidget* m_usersTable;
    QPushButton* m_addUserBtn;
    QPushButton* m_removeUserBtn;
    QPushButton* m_editUserBtn;
    
    QWidget* m_securityTab;
    QListWidget* m_allowedIpsList;
    QListWidget* m_deniedIpsList;
    QLineEdit* m_allowedIpEdit;
    QLineEdit* m_deniedIpEdit;
    
    QWidget* m_serviceTab;
    QLabel* m_serviceStatusLabel;
    QLabel* m_serviceEnabledLabel;
    QPushButton* m_installServiceBtn;
    QPushButton* m_uninstallServiceBtn;
    QPushButton* m_startServiceBtn;
    QPushButton* m_stopServiceBtn;
    QPushButton* m_enableServiceBtn;
    QPushButton* m_disableServiceBtn;
    
    QWidget* m_logsTab;
    QTextEdit* m_logView;
    QTimer* m_logRefreshTimer;
};

#endif
