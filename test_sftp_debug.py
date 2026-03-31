#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
SFTP 功能手动测试脚本 - 用于调试 CHMOD 和 SYMLINK 问题
"""

import paramiko
import stat
import os
import sys

def test_chmod():
    """测试 CHMOD 功能"""
    print("\n" + "="*70)
    print("测试 CHMOD 权限修改")
    print("="*70)
    
    ssh = paramiko.SSHClient()
    ssh.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    ssh.connect('127.0.0.1', port=2222, username='123', password='123456')
    sftp = ssh.open_sftp()
    
    try:
        # 创建文件
        filename = 'test_chmod_perms.txt'
        with sftp.file(filename, 'w') as f:
            f.write('test content for chmod')
        
        # 尝试设置为 600
        print(f"\n1. 设置 {filename} 权限为 0o600...")
        sftp.chmod(filename, 0o600)
        stat_result = sftp.stat(filename)
        mode_600 = stat_result.st_mode & 0o777
        print(f"   实际权限：{oct(mode_600)}")
        
        # 再设置为 644
        print(f"\n2. 设置 {filename} 权限为 0o644...")
        sftp.chmod(filename, 0o644)
        stat_result = sftp.stat(filename)
        mode_644 = stat_result.st_mode & 0o777
        print(f"   实际权限：{oct(mode_644)}")
        
        # 验证
        if mode_600 == 0o600 and mode_644 == 0o644:
            print(f"\n✓ CHMOD 测试通过！")
            print(f"  600 -> {oct(mode_600)}, 644 -> {oct(mode_644)}")
            return True
        else:
            print(f"\n✗ CHMOD 测试失败！")
            print(f"  期望：600 -> 0o600, 644 -> 0o644")
            print(f"  实际：600 -> {oct(mode_600)}, 644 -> {oct(mode_644)}")
            return False
            
    finally:
        # 清理
        try:
            sftp.remove(filename)
        except:
            pass
        sftp.close()
        ssh.close()


def test_symlink():
    """测试 SYMLINK 功能"""
    print("\n" + "="*70)
    print("测试 SYMLINK 符号链接")
    print("="*70)
    
    ssh = paramiko.SSHClient()
    ssh.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    ssh.connect('127.0.0.1', port=2222, username='123', password='123456')
    sftp = ssh.open_sftp()
    
    try:
        target_path = '/test_symlink_target.txt'
        link_path = '/test_symlink_link.txt'
        
        # 清理旧文件
        for f in [target_path, link_path]:
            try:
                sftp.remove(f)
            except:
                pass
        
        # 创建目标文件
        print(f"\n1. 创建目标文件 {target_path}...")
        with sftp.file(target_path, 'w') as f:
            f.write('symlink target content')
        
        # 验证目标文件存在
        try:
            sftp.stat(target_path)
            print(f"   ✓ 目标文件已创建")
        except FileNotFoundError:
            print(f"   ✗ 目标文件不存在！")
            return False
        
        # 创建符号链接
        print(f"\n2. 创建符号链接 {link_path} -> {target_path}...")
        try:
            sftp.symlink(target_path, link_path)
            print(f"   ✓ symlink() 调用成功")
        except Exception as e:
            print(f"   ✗ symlink() 调用失败：{e}")
            return False
        
        # 验证符号链接
        print(f"\n3. 验证符号链接...")
        try:
            lstat_result = sftp.lstat(link_path)
            mode = lstat_result.st_mode
            is_symlink = stat.S_ISLNK(mode)
            
            print(f"   文件类型：{oct(mode)}")
            print(f"   是否是符号链接：{is_symlink}")
            
            if is_symlink:
                print(f"\n✓ SYMLINK 测试通过！")
                
                # 测试 readlink
                print(f"\n4. 测试读取符号链接...")
                try:
                    read_target = sftp.readlink(link_path)
                    print(f"   读取结果：{read_target}")
                    if read_target == target_path:
                        print(f"   ✓ READLINK 测试通过！")
                        return True
                    else:
                        print(f"   ✗ READLINK 测试失败：目标不匹配")
                        return False
                except Exception as e:
                    print(f"   ✗ READLINK 测试失败：{e}")
                    return False
            else:
                print(f"\n✗ SYMLINK 测试失败：创建的不是符号链接")
                
                # 检查服务器端文件
                result = os.system(f'ls -la /home/wftpg/123{link_path} 2>&1 | head -1')
                return False
                
        except Exception as e:
            print(f"   ✗ 验证失败：{e}")
            return False
            
    finally:
        # 清理
        for f in [target_path, link_path]:
            try:
                sftp.remove(f)
            except:
                pass
        sftp.close()
        ssh.close()


def test_rename():
    """测试 RENAME 功能"""
    print("\n" + "="*70)
    print("测试 RENAME 重命名")
    print("="*70)
    
    ssh = paramiko.SSHClient()
    ssh.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    ssh.connect('127.0.0.1', port=2222, username='123', password='123456')
    sftp = ssh.open_sftp()
    
    try:
        old_name = '/test_rename_old.txt'
        new_name = '/test_rename_new.txt'
        
        # 清理旧文件
        for f in [old_name, new_name]:
            try:
                sftp.remove(f)
            except:
                pass
        
        # 创建文件
        print(f"\n1. 创建文件 {old_name}...")
        with sftp.file(old_name, 'w') as f:
            f.write('rename test content')
        
        # 验证文件存在
        try:
            sftp.stat(old_name)
            print(f"   ✓ 源文件已创建")
        except FileNotFoundError:
            print(f"   ✗ 源文件不存在！")
            return False
        
        # 重命名
        print(f"\n2. 重命名 {old_name} -> {new_name}...")
        try:
            sftp.rename(old_name, new_name)
            print(f"   ✓ rename() 调用成功")
        except Exception as e:
            print(f"   ✗ rename() 调用失败：{e}")
            return False
        
        # 验证重命名
        print(f"\n3. 验证重命名结果...")
        try:
            # 新文件应该存在
            sftp.stat(new_name)
            print(f"   ✓ 新文件存在")
            
            # 旧文件应该不存在
            try:
                sftp.stat(old_name)
                print(f"   ✗ 旧文件仍然存在！")
                return False
            except FileNotFoundError:
                print(f"   ✓ 旧文件已删除")
            
            print(f"\n✓ RENAME 测试通过！")
            return True
            
        except Exception as e:
            print(f"   ✗ 验证失败：{e}")
            return False
            
    finally:
        # 清理
        for f in [old_name, new_name]:
            try:
                sftp.remove(f)
            except:
                pass
        sftp.close()
        ssh.close()


if __name__ == '__main__':
    print("\n" + "="*70)
    print("WFTPG SFTP 功能调试测试")
    print("="*70)
    
    results = {}
    
    # 运行测试
    results['CHMOD'] = test_chmod()
    results['SYMLINK'] = test_symlink()
    results['READLINK'] = 'SYMLINK' in results and results['SYMLINK']
    results['RENAME'] = test_rename()
    
    # 汇总结果
    print("\n" + "="*70)
    print("测试结果汇总")
    print("="*70)
    
    for test_name, passed in results.items():
        status = "✓ 通过" if passed else "✗ 失败"
        print(f"  {test_name}: {status}")
    
    total = len(results)
    passed = sum(1 for v in results.values() if v)
    success_rate = (passed / total * 100) if total > 0 else 0
    
    print(f"\n总计：{passed}/{total} ({success_rate:.1f}%)")
    print("="*70)
    
    sys.exit(0 if all(results.values()) else 1)
