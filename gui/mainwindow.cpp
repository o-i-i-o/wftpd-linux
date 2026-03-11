#include "mainwindow.h"
#include <QVBoxLayout>
#include <QHBoxLayout>
#include <QFormLayout>
#include <QGridLayout>
#include <QHeaderView>
#include <QMessageBox>
#include <QFileDialog>
#include <QDialog>
#include <QDialogButtonBox>
#include <QComboBox>

MainWindow::MainWindow(QWidget* parent)
    : QMainWindow(parent)
    , m_bridge(new WftpgBridge(this))
{
    setupUi();
    loadConfig();
    loadUsers();
    updateServerStatus();
    updateServiceStatus();
    
    connect(m_bridge, &WftpgBridge::ftpStatusChanged, this, &MainWindow::onFtpStatusChanged);
    connect(m_bridge, &WftpgBridge::sftpStatusChanged, this, &MainWindow::onSftpStatusChanged);
    
    m_logRefreshTimer = new QTimer(this);
    connect(m_logRefreshTimer, &QTimer::timeout, this, &MainWindow::onRefreshLogs);
    m_logRefreshTimer->start(2000);
}

MainWindow::~MainWindow()
{
}

void MainWindow::setupUi()
{
    setWindowTitle(tr("WFTPG - SFTP/FTP 服务器管理工具"));
    resize(900, 600);
    
    m_tabWidget = new QTabWidget(this);
    setCentralWidget(m_tabWidget);
    
    setupServerTab();
    setupUsersTab();
    setupSecurityTab();
    setupServiceTab();
    setupLogsTab();
}

void MainWindow::setupServerTab()
{
    m_serverTab = new QWidget();
    QVBoxLayout* mainLayout = new QVBoxLayout(m_serverTab);
    
    QGroupBox* configGroup = new QGroupBox(tr("服务器配置"));
    QFormLayout* configLayout = new QFormLayout(configGroup);
    
    m_bindIpEdit = new QLineEdit();
    m_bindIpEdit->setPlaceholderText(tr("例如: 0.0.0.0"));
    configLayout->addRow(tr("绑定IP:"), m_bindIpEdit);
    
    QHBoxLayout* portLayout = new QHBoxLayout();
    m_ftpPortSpin = new QSpinBox();
    m_ftpPortSpin->setRange(1, 65535);
    m_ftpPortSpin->setValue(21);
    portLayout->addWidget(new QLabel(tr("FTP端口:")));
    portLayout->addWidget(m_ftpPortSpin);
    
    m_sftpPortSpin = new QSpinBox();
    m_sftpPortSpin->setRange(1, 65535);
    m_sftpPortSpin->setValue(22);
    portLayout->addWidget(new QLabel(tr("SFTP端口:")));
    portLayout->addWidget(m_sftpPortSpin);
    portLayout->addStretch();
    
    configLayout->addRow(tr("端口设置:"), portLayout);
    
    QHBoxLayout* enabledLayout = new QHBoxLayout();
    m_ftpEnabledCheck = new QCheckBox(tr("启用FTP"));
    m_sftpEnabledCheck = new QCheckBox(tr("启用SFTP"));
    enabledLayout->addWidget(m_ftpEnabledCheck);
    enabledLayout->addWidget(m_sftpEnabledCheck);
    enabledLayout->addStretch();
    configLayout->addRow(tr("服务开关:"), enabledLayout);
    
    QHBoxLayout* ftpHomeLayout = new QHBoxLayout();
    m_ftpHomeEdit = new QLineEdit();
    QPushButton* browseFtpBtn = new QPushButton(tr("浏览..."));
    connect(browseFtpBtn, &QPushButton::clicked, this, &MainWindow::onBrowseFtpHome);
    ftpHomeLayout->addWidget(m_ftpHomeEdit);
    ftpHomeLayout->addWidget(browseFtpBtn);
    configLayout->addRow(tr("FTP主目录:"), ftpHomeLayout);
    
    QHBoxLayout* sftpHomeLayout = new QHBoxLayout();
    m_sftpHomeEdit = new QLineEdit();
    QPushButton* browseSftpBtn = new QPushButton(tr("浏览..."));
    connect(browseSftpBtn, &QPushButton::clicked, this, &MainWindow::onBrowseSftpHome);
    sftpHomeLayout->addWidget(m_sftpHomeEdit);
    sftpHomeLayout->addWidget(browseSftpBtn);
    configLayout->addRow(tr("SFTP主目录:"), sftpHomeLayout);
    
    QPushButton* saveConfigBtn = new QPushButton(tr("保存配置"));
    connect(saveConfigBtn, &QPushButton::clicked, this, &MainWindow::onConfigSaveClicked);
    configLayout->addRow(QString(), saveConfigBtn);
    
    mainLayout->addWidget(configGroup);
    
    QGroupBox* controlGroup = new QGroupBox(tr("服务控制"));
    QGridLayout* controlLayout = new QGridLayout(controlGroup);
    
    controlLayout->addWidget(new QLabel(tr("FTP服务:")), 0, 0);
    m_ftpStartBtn = new QPushButton(tr("启动"));
    m_ftpStopBtn = new QPushButton(tr("停止"));
    m_ftpStatusLabel = new QLabel(tr("已停止"));
    m_ftpStatusLabel->setStyleSheet("color: red;");
    controlLayout->addWidget(m_ftpStartBtn, 0, 1);
    controlLayout->addWidget(m_ftpStopBtn, 0, 2);
    controlLayout->addWidget(m_ftpStatusLabel, 0, 3);
    
    controlLayout->addWidget(new QLabel(tr("SFTP服务:")), 1, 0);
    m_sftpStartBtn = new QPushButton(tr("启动"));
    m_sftpStopBtn = new QPushButton(tr("停止"));
    m_sftpStatusLabel = new QLabel(tr("已停止"));
    m_sftpStatusLabel->setStyleSheet("color: red;");
    controlLayout->addWidget(m_sftpStartBtn, 1, 1);
    controlLayout->addWidget(m_sftpStopBtn, 1, 2);
    controlLayout->addWidget(m_sftpStatusLabel, 1, 3);
    
    connect(m_ftpStartBtn, &QPushButton::clicked, this, &MainWindow::onFtpStartClicked);
    connect(m_ftpStopBtn, &QPushButton::clicked, this, &MainWindow::onFtpStopClicked);
    connect(m_sftpStartBtn, &QPushButton::clicked, this, &MainWindow::onSftpStartClicked);
    connect(m_sftpStopBtn, &QPushButton::clicked, this, &MainWindow::onSftpStopClicked);
    
    mainLayout->addWidget(controlGroup);
    mainLayout->addStretch();
    
    m_tabWidget->addTab(m_serverTab, tr("服务器"));
}

void MainWindow::setupUsersTab()
{
    m_usersTab = new QWidget();
    QVBoxLayout* mainLayout = new QVBoxLayout(m_usersTab);
    
    m_usersTable = new QTableWidget();
    m_usersTable->setColumnCount(5);
    m_usersTable->setHorizontalHeaderLabels(QStringList() 
        << tr("用户名") << tr("主目录") << tr("状态") << tr("管理员") << tr("权限"));
    m_usersTable->horizontalHeader()->setStretchLastSection(true);
    m_usersTable->setSelectionBehavior(QAbstractItemView::SelectRows);
    m_usersTable->setEditTriggers(QAbstractItemView::NoEditTriggers);
    
    mainLayout->addWidget(m_usersTable);
    
    QHBoxLayout* btnLayout = new QHBoxLayout();
    m_addUserBtn = new QPushButton(tr("添加用户"));
    m_removeUserBtn = new QPushButton(tr("删除用户"));
    m_editUserBtn = new QPushButton(tr("编辑用户"));
    
    connect(m_addUserBtn, &QPushButton::clicked, this, &MainWindow::onUserAddClicked);
    connect(m_removeUserBtn, &QPushButton::clicked, this, &MainWindow::onUserRemoveClicked);
    connect(m_editUserBtn, &QPushButton::clicked, this, &MainWindow::onUserEditClicked);
    
    btnLayout->addWidget(m_addUserBtn);
    btnLayout->addWidget(m_removeUserBtn);
    btnLayout->addWidget(m_editUserBtn);
    btnLayout->addStretch();
    
    mainLayout->addLayout(btnLayout);
    
    m_tabWidget->addTab(m_usersTab, tr("用户管理"));
}

void MainWindow::setupSecurityTab()
{
    m_securityTab = new QWidget();
    QHBoxLayout* mainLayout = new QHBoxLayout(m_securityTab);
    
    QGroupBox* allowedGroup = new QGroupBox(tr("允许的IP地址"));
    QVBoxLayout* allowedLayout = new QVBoxLayout(allowedGroup);
    
    m_allowedIpsList = new QListWidget();
    allowedLayout->addWidget(m_allowedIpsList);
    
    QHBoxLayout* allowedBtnLayout = new QHBoxLayout();
    m_allowedIpEdit = new QLineEdit();
    m_allowedIpEdit->setPlaceholderText(tr("例如: 192.168.1.0/24"));
    QPushButton* addAllowedBtn = new QPushButton(tr("添加"));
    QPushButton* removeAllowedBtn = new QPushButton(tr("删除"));
    connect(addAllowedBtn, &QPushButton::clicked, this, &MainWindow::onIpAddAllowedClicked);
    connect(removeAllowedBtn, &QPushButton::clicked, this, &MainWindow::onIpRemoveAllowedClicked);
    allowedBtnLayout->addWidget(m_allowedIpEdit);
    allowedBtnLayout->addWidget(addAllowedBtn);
    allowedBtnLayout->addWidget(removeAllowedBtn);
    allowedLayout->addLayout(allowedBtnLayout);
    
    mainLayout->addWidget(allowedGroup);
    
    QGroupBox* deniedGroup = new QGroupBox(tr("禁止的IP地址"));
    QVBoxLayout* deniedLayout = new QVBoxLayout(deniedGroup);
    
    m_deniedIpsList = new QListWidget();
    deniedLayout->addWidget(m_deniedIpsList);
    
    QHBoxLayout* deniedBtnLayout = new QHBoxLayout();
    m_deniedIpEdit = new QLineEdit();
    m_deniedIpEdit->setPlaceholderText(tr("例如: 10.0.0.0/8"));
    QPushButton* addDeniedBtn = new QPushButton(tr("添加"));
    QPushButton* removeDeniedBtn = new QPushButton(tr("删除"));
    connect(addDeniedBtn, &QPushButton::clicked, this, &MainWindow::onIpAddDeniedClicked);
    connect(removeDeniedBtn, &QPushButton::clicked, this, &MainWindow::onIpRemoveDeniedClicked);
    deniedBtnLayout->addWidget(m_deniedIpEdit);
    deniedBtnLayout->addWidget(addDeniedBtn);
    deniedBtnLayout->addWidget(removeDeniedBtn);
    deniedLayout->addLayout(deniedBtnLayout);
    
    mainLayout->addWidget(deniedGroup);
    
    m_tabWidget->addTab(m_securityTab, tr("安全设置"));
}

void MainWindow::setupServiceTab()
{
    m_serviceTab = new QWidget();
    QVBoxLayout* mainLayout = new QVBoxLayout(m_serviceTab);
    
    QGroupBox* statusGroup = new QGroupBox(tr("服务状态"));
    QFormLayout* statusLayout = new QFormLayout(statusGroup);
    
    m_serviceStatusLabel = new QLabel(tr("未运行"));
    m_serviceEnabledLabel = new QLabel(tr("未启用"));
    statusLayout->addRow(tr("运行状态:"), m_serviceStatusLabel);
    statusLayout->addRow(tr("开机启动:"), m_serviceEnabledLabel);
    
    mainLayout->addWidget(statusGroup);
    
    QGroupBox* controlGroup = new QGroupBox(tr("服务管理"));
    QGridLayout* controlLayout = new QGridLayout(controlGroup);
    
    m_installServiceBtn = new QPushButton(tr("安装服务"));
    m_uninstallServiceBtn = new QPushButton(tr("卸载服务"));
    m_startServiceBtn = new QPushButton(tr("启动服务"));
    m_stopServiceBtn = new QPushButton(tr("停止服务"));
    m_enableServiceBtn = new QPushButton(tr("启用开机启动"));
    m_disableServiceBtn = new QPushButton(tr("禁用开机启动"));
    
    connect(m_installServiceBtn, &QPushButton::clicked, this, &MainWindow::onServiceInstallClicked);
    connect(m_uninstallServiceBtn, &QPushButton::clicked, this, &MainWindow::onServiceUninstallClicked);
    connect(m_startServiceBtn, &QPushButton::clicked, this, &MainWindow::onServiceStartClicked);
    connect(m_stopServiceBtn, &QPushButton::clicked, this, &MainWindow::onServiceStopClicked);
    connect(m_enableServiceBtn, &QPushButton::clicked, this, &MainWindow::onServiceEnableClicked);
    connect(m_disableServiceBtn, &QPushButton::clicked, this, &MainWindow::onServiceDisableClicked);
    
    controlLayout->addWidget(m_installServiceBtn, 0, 0);
    controlLayout->addWidget(m_uninstallServiceBtn, 0, 1);
    controlLayout->addWidget(m_startServiceBtn, 1, 0);
    controlLayout->addWidget(m_stopServiceBtn, 1, 1);
    controlLayout->addWidget(m_enableServiceBtn, 2, 0);
    controlLayout->addWidget(m_disableServiceBtn, 2, 1);
    
    mainLayout->addWidget(controlGroup);
    mainLayout->addStretch();
    
    m_tabWidget->addTab(m_serviceTab, tr("系统服务"));
}

void MainWindow::setupLogsTab()
{
    m_logsTab = new QWidget();
    QVBoxLayout* mainLayout = new QVBoxLayout(m_logsTab);
    
    m_logView = new QTextEdit();
    m_logView->setReadOnly(true);
    m_logView->setFont(QFont("monospace"));
    
    mainLayout->addWidget(m_logView);
    
    m_tabWidget->addTab(m_logsTab, tr("日志查看"));
}

void MainWindow::loadConfig()
{
    m_bindIpEdit->setText(m_bridge->getBindIp());
    m_ftpPortSpin->setValue(m_bridge->getFtpPort());
    m_sftpPortSpin->setValue(m_bridge->getSftpPort());
    m_ftpEnabledCheck->setChecked(m_bridge->isFtpEnabled());
    m_sftpEnabledCheck->setChecked(m_bridge->isSftpEnabled());
    m_ftpHomeEdit->setText(m_bridge->getFtpHome());
    m_sftpHomeEdit->setText(m_bridge->getSftpHome());
}

void MainWindow::loadUsers()
{
    m_usersTable->setRowCount(0);
}

void MainWindow::updateServerStatus()
{
    bool ftpRunning = m_bridge->isFtpRunning();
    bool sftpRunning = m_bridge->isSftpRunning();
    
    m_ftpStatusLabel->setText(ftpRunning ? tr("运行中") : tr("已停止"));
    m_ftpStatusLabel->setStyleSheet(ftpRunning ? "color: green;" : "color: red;");
    m_ftpStartBtn->setEnabled(!ftpRunning);
    m_ftpStopBtn->setEnabled(ftpRunning);
    
    m_sftpStatusLabel->setText(sftpRunning ? tr("运行中") : tr("已停止"));
    m_sftpStatusLabel->setStyleSheet(sftpRunning ? "color: green;" : "color: red;");
    m_sftpStartBtn->setEnabled(!sftpRunning);
    m_sftpStopBtn->setEnabled(sftpRunning);
}

void MainWindow::updateServiceStatus()
{
    bool running = m_bridge->isServiceRunning();
    bool enabled = m_bridge->isServiceEnabled();
    bool exists = m_bridge->serviceExists();
    
    m_serviceStatusLabel->setText(running ? tr("运行中") : tr("已停止"));
    m_serviceStatusLabel->setStyleSheet(running ? "color: green;" : "color: red;");
    m_serviceEnabledLabel->setText(enabled ? tr("已启用") : tr("未启用"));
    m_serviceEnabledLabel->setStyleSheet(enabled ? "color: green;" : "color: gray;");
    
    m_installServiceBtn->setEnabled(!exists);
    m_uninstallServiceBtn->setEnabled(exists);
    m_startServiceBtn->setEnabled(exists && !running);
    m_stopServiceBtn->setEnabled(exists && running);
    m_enableServiceBtn->setEnabled(exists && !enabled);
    m_disableServiceBtn->setEnabled(exists && enabled);
}

void MainWindow::onFtpStartClicked()
{
    if (m_bridge->startFtp()) {
        QMessageBox::information(this, tr("成功"), tr("FTP服务已启动"));
    } else {
        QMessageBox::warning(this, tr("失败"), tr("无法启动FTP服务"));
    }
    updateServerStatus();
}

void MainWindow::onFtpStopClicked()
{
    m_bridge->stopFtp();
    QMessageBox::information(this, tr("成功"), tr("FTP服务已停止"));
    updateServerStatus();
}

void MainWindow::onSftpStartClicked()
{
    if (m_bridge->startSftp()) {
        QMessageBox::information(this, tr("成功"), tr("SFTP服务已启动"));
    } else {
        QMessageBox::warning(this, tr("失败"), tr("无法启动SFTP服务"));
    }
    updateServerStatus();
}

void MainWindow::onSftpStopClicked()
{
    m_bridge->stopSftp();
    QMessageBox::information(this, tr("成功"), tr("SFTP服务已停止"));
    updateServerStatus();
}

void MainWindow::onConfigSaveClicked()
{
    if (m_bridge->setConfig(
        m_bindIpEdit->text(),
        m_ftpPortSpin->value(),
        m_sftpPortSpin->value(),
        m_ftpEnabledCheck->isChecked(),
        m_sftpEnabledCheck->isChecked(),
        m_ftpHomeEdit->text(),
        m_sftpHomeEdit->text()
    )) {
        QMessageBox::information(this, tr("成功"), tr("配置已保存"));
    } else {
        QMessageBox::warning(this, tr("失败"), tr("保存配置失败"));
    }
}

void MainWindow::onUserAddClicked()
{
    QDialog dialog(this);
    dialog.setWindowTitle(tr("添加用户"));
    QFormLayout* layout = new QFormLayout(&dialog);
    
    QLineEdit* usernameEdit = new QLineEdit();
    QLineEdit* passwordEdit = new QLineEdit();
    passwordEdit->setEchoMode(QLineEdit::Password);
    QLineEdit* homeEdit = new QLineEdit();
    homeEdit->setText(m_bridge->getFtpHome());
    QCheckBox* adminCheck = new QCheckBox();
    
    layout->addRow(tr("用户名:"), usernameEdit);
    layout->addRow(tr("密码:"), passwordEdit);
    layout->addRow(tr("主目录:"), homeEdit);
    layout->addRow(tr("管理员:"), adminCheck);
    
    QDialogButtonBox* buttons = new QDialogButtonBox(
        QDialogButtonBox::Ok | QDialogButtonBox::Cancel);
    connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
    connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
    layout->addRow(buttons);
    
    if (dialog.exec() == QDialog::Accepted) {
        if (m_bridge->addUser(usernameEdit->text(), passwordEdit->text(),
                              homeEdit->text(), adminCheck->isChecked())) {
            QMessageBox::information(this, tr("成功"), tr("用户已添加"));
            loadUsers();
        } else {
            QMessageBox::warning(this, tr("失败"), tr("添加用户失败"));
        }
    }
}

void MainWindow::onUserRemoveClicked()
{
    int row = m_usersTable->currentRow();
    if (row < 0) {
        QMessageBox::warning(this, tr("提示"), tr("请先选择要删除的用户"));
        return;
    }
    
    QString username = m_usersTable->item(row, 0)->text();
    if (QMessageBox::question(this, tr("确认"), tr("确定要删除用户 %1 吗?").arg(username))
        == QMessageBox::Yes) {
        if (m_bridge->removeUser(username)) {
            QMessageBox::information(this, tr("成功"), tr("用户已删除"));
            loadUsers();
        } else {
            QMessageBox::warning(this, tr("失败"), tr("删除用户失败"));
        }
    }
}

void MainWindow::onUserEditClicked()
{
    int row = m_usersTable->currentRow();
    if (row < 0) {
        QMessageBox::warning(this, tr("提示"), tr("请先选择要编辑的用户"));
        return;
    }
    
    QString username = m_usersTable->item(row, 0)->text();
    
    QDialog dialog(this);
    dialog.setWindowTitle(tr("编辑用户 - %1").arg(username));
    QFormLayout* layout = new QFormLayout(&dialog);
    
    QLineEdit* passwordEdit = new QLineEdit();
    passwordEdit->setPlaceholderText(tr("留空则不修改密码"));
    passwordEdit->setEchoMode(QLineEdit::Password);
    QLineEdit* homeEdit = new QLineEdit();
    homeEdit->setText(m_usersTable->item(row, 1)->text());
    QCheckBox* enabledCheck = new QCheckBox();
    enabledCheck->setChecked(m_usersTable->item(row, 2)->text() == tr("启用"));
    
    layout->addRow(tr("新密码:"), passwordEdit);
    layout->addRow(tr("主目录:"), homeEdit);
    layout->addRow(tr("启用:"), enabledCheck);
    
    QDialogButtonBox* buttons = new QDialogButtonBox(
        QDialogButtonBox::Ok | QDialogButtonBox::Cancel);
    connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
    connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);
    layout->addRow(buttons);
    
    if (dialog.exec() == QDialog::Accepted) {
        if (!passwordEdit->text().isEmpty()) {
            m_bridge->updateUserPassword(username, passwordEdit->text());
        }
        m_bridge->setUserEnabled(username, enabledCheck->isChecked());
        QMessageBox::information(this, tr("成功"), tr("用户信息已更新"));
        loadUsers();
    }
}

void MainWindow::onIpAddAllowedClicked()
{
    QString ip = m_allowedIpEdit->text().trimmed();
    if (ip.isEmpty()) return;
    
    if (m_bridge->addAllowedIp(ip)) {
        m_allowedIpsList->addItem(ip);
        m_allowedIpEdit->clear();
    }
}

void MainWindow::onIpRemoveAllowedClicked()
{
    QListWidgetItem* item = m_allowedIpsList->currentItem();
    if (!item) return;
    
    QString ip = item->text();
    if (m_bridge->removeAllowedIp(ip)) {
        delete item;
    }
}

void MainWindow::onIpAddDeniedClicked()
{
    QString ip = m_deniedIpEdit->text().trimmed();
    if (ip.isEmpty()) return;
    
    if (m_bridge->addDeniedIp(ip)) {
        m_deniedIpsList->addItem(ip);
        m_deniedIpEdit->clear();
    }
}

void MainWindow::onIpRemoveDeniedClicked()
{
    QListWidgetItem* item = m_deniedIpsList->currentItem();
    if (!item) return;
    
    QString ip = item->text();
    if (m_bridge->removeDeniedIp(ip)) {
        delete item;
    }
}

void MainWindow::onServiceInstallClicked()
{
    QString binaryPath = QCoreApplication::applicationFilePath();
    if (m_bridge->installService(binaryPath)) {
        QMessageBox::information(this, tr("成功"), tr("服务已安装"));
    } else {
        QMessageBox::warning(this, tr("失败"), tr("安装服务失败，请以root权限运行"));
    }
    updateServiceStatus();
}

void MainWindow::onServiceUninstallClicked()
{
    if (m_bridge->uninstallService()) {
        QMessageBox::information(this, tr("成功"), tr("服务已卸载"));
    } else {
        QMessageBox::warning(this, tr("失败"), tr("卸载服务失败"));
    }
    updateServiceStatus();
}

void MainWindow::onServiceStartClicked()
{
    if (m_bridge->startService()) {
        QMessageBox::information(this, tr("成功"), tr("服务已启动"));
    } else {
        QMessageBox::warning(this, tr("失败"), tr("启动服务失败"));
    }
    updateServiceStatus();
}

void MainWindow::onServiceStopClicked()
{
    if (m_bridge->stopService()) {
        QMessageBox::information(this, tr("成功"), tr("服务已停止"));
    } else {
        QMessageBox::warning(this, tr("失败"), tr("停止服务失败"));
    }
    updateServiceStatus();
}

void MainWindow::onServiceEnableClicked()
{
    if (m_bridge->enableService()) {
        QMessageBox::information(this, tr("成功"), tr("已启用开机启动"));
    } else {
        QMessageBox::warning(this, tr("失败"), tr("启用开机启动失败"));
    }
    updateServiceStatus();
}

void MainWindow::onServiceDisableClicked()
{
    if (m_bridge->disableService()) {
        QMessageBox::information(this, tr("成功"), tr("已禁用开机启动"));
    } else {
        QMessageBox::warning(this, tr("失败"), tr("禁用开机启动失败"));
    }
    updateServiceStatus();
}

void MainWindow::onRefreshLogs()
{
}

void MainWindow::onFtpStatusChanged(bool running)
{
    Q_UNUSED(running);
    updateServerStatus();
}

void MainWindow::onSftpStatusChanged(bool running)
{
    Q_UNUSED(running);
    updateServerStatus();
}

void MainWindow::onBrowseFtpHome()
{
    QString dir = QFileDialog::getExistingDirectory(this, tr("选择FTP主目录"));
    if (!dir.isEmpty()) {
        m_ftpHomeEdit->setText(dir);
    }
}

void MainWindow::onBrowseSftpHome()
{
    QString dir = QFileDialog::getExistingDirectory(this, tr("选择SFTP主目录"));
    if (!dir.isEmpty()) {
        m_sftpHomeEdit->setText(dir);
    }
}

void MainWindow::addLogEntry(const QString& timestamp, const QString& level,
                              const QString& source, const QString& message, const QString& clientIp)
{
    QString logLine = QString("[%1] [%2] [%3] %4 - IP: %5")
        .arg(timestamp)
        .arg(level)
        .arg(source)
        .arg(message)
        .arg(clientIp);
    
    m_logView->append(logLine);
}
