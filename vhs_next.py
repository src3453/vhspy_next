import cv2
import numpy as np
import scipy.ndimage as ndimage
import scipy.signal as signal
import math
import time
import sys
import random

# Webカメラを使うときはこちら
cap = cv2.VideoCapture(5)
cap.set(cv2.CAP_PROP_FRAME_WIDTH, 1280)
cap.set(cv2.CAP_PROP_FRAME_HEIGHT, 720)
cap.set(cv2.CAP_PROP_FPS, 60)
avg = None

cv2.namedWindow("Main",cv2.WINDOW_FREERATIO)

#FRAME_WIDTH = 933
#FRAME_HEIGHT = 525
FRAME_WIDTH = 480
FRAME_HEIGHT = 360
COLOR_GAIN = 1.0  # 色の強さを調整するための係数
BORDER_SIZE = 16
NOISE_LEVEL = 16  # ノイズの強さを調整するための係数
DOT_CRAWL_GAIN = 1.0 # ドットクローリングの強さを調整するための係数
#__frame = cv2.imread(r"C:\Users\y2k34\Downloads\ERgNUALUwAE-tXU.png")
#FRAME_WIDTH, FRAME_HEIGHT = __frame.shape[1], __frame.shape[0]

lowpass_filter_y = signal.butter(6, 0.5, btype='lowpass', output='sos', analog=False) # prepare lowpass filter for Y
lowpass_filter_c = signal.butter(4, 0.125, btype='lowpass', output='sos', analog=False) # prepare lowpass filter for Cb/Cr
highpass_filter_c = signal.butter(4, 0.5, btype='highpass', output='sos', analog=False) # prepare highpass filter for Cb/Cr
#highpass_filter_yc = signal.butter(32, 0.001, btype='highpass', output='sos', analog=False) # prepare highpass filter for Y, Cb/Cr

#bandpass_filter_y = signal.butter(1, (0.5,0.99), btype='bandpass', output='sos') # prepare bandpass filter for Y to simulate color crosstalking
# make cos/sin mask for simulating cross color
mask_cr = -np.sin(np.linspace(0, np.pi*FRAME_WIDTH/2, FRAME_WIDTH+1)).astype(np.float32)[:-1] # prepare color mask for Cr
mask_cr = np.repeat(mask_cr[np.newaxis, :], FRAME_HEIGHT, axis=0) # repeat mask for Cr
mask_cb = np.cos(np.linspace(0, np.pi*FRAME_WIDTH/2, FRAME_WIDTH+1)).astype(np.float32)[:-1] # prepare color mask for Cb
mask_cb = np.repeat(mask_cb[np.newaxis, :], FRAME_HEIGHT, axis=0) # repeat mask for Cb
mask_y = (np.arange(FRAME_WIDTH)/2)%2
mask_y_roll = np.arange(FRAME_HEIGHT)
mask_y = ((np.repeat(mask_y[np.newaxis, :], FRAME_HEIGHT, axis=0)+np.repeat(mask_y_roll[:, np.newaxis], FRAME_WIDTH, axis=1))%2).astype(float) # prepare mask for Y to simulate interlaced video

# 事前に平坦化・正規化したマスクを用意
mask_cr_flat = mask_cr.flatten()
mask_cb_flat = mask_cb.flatten()
mask_y_flat = (mask_y.flatten() * 2 - 1) * 255.0

mask_cr_flat_cg = mask_cr_flat * COLOR_GAIN
mask_cb_flat_cg = mask_cb_flat * COLOR_GAIN
mask_y_flat_half = mask_y_flat * 0.5

# グローバル輝度調整用のLUT
brightness_lut = np.clip(np.arange(256, dtype=np.float32) * 0.85 + 0.05, 0, 255).astype(np.uint8)
TANH_FACTOR = 1.5
hysteresis_lut = np.tanh(np.linspace(-TANH_FACTOR, TANH_FACTOR, 256, dtype=np.float32)) * 128 + 128  # ヒステリシスのルックアップテーブル (非線形な応答をシミュレート)
hysteresis_lut = np.clip(hysteresis_lut, 0, 255).astype(np.uint8)

# 乱数バッファの事前生成 (計算量削減のため)
# フレームサイズ以上の十分大きなノイズバッファを用意し、毎フレームスライスして使う
_noise_buffer_size = FRAME_WIDTH * FRAME_HEIGHT * 3 * 2
_noise_buffer = np.random.uniform(-NOISE_LEVEL/2, NOISE_LEVEL/2, (_noise_buffer_size, 3)).astype(np.float32)
_noise_offset = 0

frame = np.zeros((FRAME_HEIGHT, FRAME_WIDTH, 3), dtype=np.uint8) # create empty frame
fnum = 0
_frame_avg = None
_dst = None
def accumulatebyMatWeight(source:cv2.Mat, dest:cv2.Mat, alpha:cv2.Mat) -> cv2.Mat:
    new_value = (1-alpha)*dest+alpha*source
    return new_value

dst_buffer = np.zeros((FRAME_HEIGHT, FRAME_WIDTH, 3), dtype=np.float32) # create buffer for processing

overall_start_time = time.time()
while True:
    start_time = time.time()
    # 1フレームずつ取得する。
    ret, _frame = cap.read()
    if not ret:
        break
    _frame = cv2.resize(_frame, (FRAME_WIDTH, FRAME_HEIGHT), interpolation=cv2.INTER_AREA)
    if _frame_avg is None:
        _frame_avg = _frame.copy().astype(np.float32)
    EXPONENT_FACTOR = 1.5 # adjust this factor to change the curve of the weighted moving average, higher value means more "comet tail" effect in only bright areas, lower value means more "comet tail" effect in dark areas
    #_frame_avg = accumulatebyMatWeight(_frame,_frame_avg,(1.0-np.power(_frame_avg/256,EXPONENT_FACTOR))) # calculate weighted moving average to simulate tube-based camera hysteresis (aka comet tail)
    #_frame = _frame_avg.copy()
    #frame = cv2.copyMakeBorder(cv2.resize(frame, (FRAME_WIDTH-BORDER_SIZE, FRAME_HEIGHT), interpolation=cv2.INTER_AREA),0,0,BORDER_SIZE//2,BORDER_SIZE//2,cv2.BORDER_CONSTANT)
    frame = _frame.astype(np.uint8)
#Place code here

    frame_ycbcr = cv2.cvtColor(frame, cv2.COLOR_BGR2YCrCb) # convert to YCrCb to process video
    
    frame_ycbcr_2d = frame_ycbcr.reshape(-1, 3) # reshape to 2d to apply continuous filtering
    dst_ycbcr_2d = frame_ycbcr_2d.astype(np.float32) # copy不要

    # ノイズのオンザフライ生成を避け、バッファから利用
    noise_len = dst_ycbcr_2d.shape[0]
    # 時々バッファを更新してパターン化を防いでもよい（適宜）
    
    # 形状を合わせて加算
    dst_ycbcr_2d += _noise_buffer[_noise_offset:_noise_offset+noise_len, :].reshape(dst_ycbcr_2d.shape)
    # ノイズの開始位置をランダムにずらす (パターン化防止のため)
    _noise_offset += random.randint(0, noise_len)
    _noise_offset %= _noise_buffer_size - noise_len  # バッファサイズを超えないように
    

    # dst_y_cの計算 (rollをスライスに置き換えてコピー削減)
    # dst_y_c = (dst_y_c-np.roll(dst_y_c,-2,axis=0)).flatten()*0.75
    y_channel = frame_ycbcr[:,:,0].astype(np.float32)
    dst_y_c_2d = np.zeros_like(y_channel)
    ROLL_OFFSET = 1
    dst_y_c_2d[:-ROLL_OFFSET, :] = (y_channel[:-ROLL_OFFSET, :] - y_channel[ROLL_OFFSET:, :]) * 0.75
    dst_y_c_2d[-ROLL_OFFSET:, :] = (y_channel[-ROLL_OFFSET:, :] - y_channel[:ROLL_OFFSET, :]) * 0.75# エッジ処理(およそroll相当)
    dst_y_c = dst_y_c_2d.flatten()

    # Cb, Crをまとめてハイパスフィルタにかける
    dst_c_diff = frame_ycbcr[:,:,1:3].reshape(-1, 2).astype(np.float32) / 255.0
    dst_c_diff = signal.sosfilt(highpass_filter_c, dst_c_diff, axis=0) # 2ch同時に処理
    dst_cb_diff = dst_c_diff[:,0]
    dst_cr_diff = dst_c_diff[:,1]
    
    dst_ycbcr_2d[:,1] += dst_y_c * mask_cr_flat_cg # apply color crosstalking (Cb)
    dst_ycbcr_2d[:,2] += dst_y_c * mask_cb_flat_cg # apply color crosstalking (Cr)
    
    dst_c_diff_abs = np.abs(dst_cb_diff) + np.abs(dst_cr_diff)
    dst_c_diff_abs *= DOT_CRAWL_GAIN
    dst_ycbcr_2d[:,0] += dst_c_diff_abs * mask_y_flat_half # apply luma crosstalking (Cb+Cr to Y)
    
    dst_ycbcr_2d[:,0] = signal.sosfilt(lowpass_filter_y, dst_ycbcr_2d[:,0]) # filtering signal (Y)
    # Cb, Crをまとめてローパスフィルタにかける
    dst_ycbcr_2d[:,1:3] = signal.sosfilt(lowpass_filter_c, dst_ycbcr_2d[:,1:3], axis=0)

    #dst_ycbcr_2d = signal.sosfilt(highpass_filter_yc, dst_ycbcr_2d, axis=0) # apply highpass filter to all channels to simulate signal decay and tearing
        
    # dst_ycbcr_2d = np.roll(dst_ycbcr_2d,-4,axis=0) # roll signal to kill signal delay
    dst_ycbcr_2d[:-4, :] = dst_ycbcr_2d[4:, :]  # rollと同等の処理(エッジは少し欠けるが高速化)をスライスで
    dst_ycbcr_2d[-4:, :] = dst_ycbcr_2d[:4, :]  # 必要最低限のエッジ処理
    
    np.clip(dst_ycbcr_2d, 0, 255, out=dst_ycbcr_2d)
    dst_ycbcr = dst_ycbcr_2d.reshape((FRAME_HEIGHT, FRAME_WIDTH, 3)).astype(np.uint8) # reshape to 3d
    dst_ycbcr = cv2.LUT(dst_ycbcr, hysteresis_lut) # apply hysteresis to simulate magnetic hysteresis

    dst = cv2.cvtColor(dst_ycbcr, cv2.COLOR_YCrCb2BGR) # convert to BGR to showing
    dst = cv2.LUT(dst, brightness_lut) # global brightness adjustment to simulate poor video signal
    
    if _dst is None:
        _dst = np.zeros_like(dst)
    
    idx = fnum % 2
    _dst[idx::2, :, :] = dst[idx::2, :, :]  # grab every second frame to simulate interlaced video (Weave deinterlace)
    #_dst = cv2.resize(dst[idx::2], (FRAME_WIDTH, FRAME_HEIGHT), interpolation=cv2.INTER_LINEAR) # simulate interlaced video by resizing every second line (Bob deinterlace)
    # simulate HSync jitter by rolling each line by a small amount (horizontal jitter)]
    #jitter = np.random.normal(0, 0.3, FRAME_HEIGHT)  # adjust this range to increase/decrease jitter strength
    #for y in range(FRAME_HEIGHT):
    #    _dst[y] = np.roll(_dst[y], int(jitter[y]), axis=0) # apply horizontal jitter by rolling each line
    #_dst = np.roll(_dst, int(jitter[0]), axis=0) # apply vertical jitter by rolling whole frame
    #print (jitter.astype(int))

    #for y in range(FRAME_HEIGHT-8, FRAME_HEIGHT):
    #    _dst[y] = np.roll(_dst[y], int(math.pow((FRAME_HEIGHT-y)/8, 2)*5), axis=0) # apply stronger jitter to bottom lines to simulate tearing
    #dst_buffer = dst.astype(np.float32)
    #print(_dst)
    #cv2.imshow("Main", dst) # show video without interlace simulation
    cv2.imshow("Main", _dst) # show video
    key = cv2.waitKey(1)
    if key == 27:
        break
    fnum += 1
    
    elapsed_time = time.time() - start_time
    fps = 1.0 / elapsed_time if elapsed_time > 0 else 0
    print(f"FPS: {fps:.2f}\r", end="")
    sys.stdout.flush()


cap.release()
cv2.destroyAllWindows()

if fnum > 0:
    total_time = time.time() - overall_start_time
    avg_fps = fnum / total_time
    print(f"Average FPS: {avg_fps:.2f}")
