/*
 * vk-clear: Vulkan work on a GPU checked byte for byte (ADR 0119).
 *
 * Picks the physical device whose name contains the first argument
 * ("Venus": Mesa's Venus driver on Poly's virtio GPU), makes a device with
 * a queue, clears a 64x64 R8G8B8A8 image to a known colour, copies it to a
 * host-visible buffer, waits on a fence and checks every pixel. Prints the
 * device and "vk-clear: ok". No shaders and no window system: only what
 * the Vulkan 1.0 core API needs.
 *
 * Built against the Khronos Vulkan headers, dynamically linked to the
 * loader (libvulkan).
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <vulkan/vulkan.h>

#define SIDE 64

#define CHECK(call)                                                                   \
    do {                                                                              \
        VkResult r_ = (call);                                                         \
        if (r_ != VK_SUCCESS) {                                                       \
            printf("vk-clear: FAIL %s returned %d\n", #call, (int)r_);                \
            return 1;                                                                 \
        }                                                                             \
    } while (0)

static int memory_type(const VkPhysicalDeviceMemoryProperties *mp, uint32_t bits, VkMemoryPropertyFlags want) {
    for (uint32_t i = 0; i < mp->memoryTypeCount; i++)
        if ((bits & (1u << i)) && (mp->memoryTypes[i].propertyFlags & want) == want)
            return (int)i;
    return -1;
}

int main(int argc, char **argv) {
    const char *want = argc > 1 ? argv[1] : "";
    VkApplicationInfo app = {.sType = VK_STRUCTURE_TYPE_APPLICATION_INFO,
                             .pApplicationName = "vk-clear",
                             .apiVersion = VK_API_VERSION_1_1};
    VkInstanceCreateInfo ici = {.sType = VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO, .pApplicationInfo = &app};
    VkInstance inst;
    CHECK(vkCreateInstance(&ici, NULL, &inst));
    uint32_t n = 0;
    CHECK(vkEnumeratePhysicalDevices(inst, &n, NULL));
    VkPhysicalDevice devs[16];
    if (n > 16)
        n = 16;
    CHECK(vkEnumeratePhysicalDevices(inst, &n, devs));
    VkPhysicalDevice pd = VK_NULL_HANDLE;
    VkPhysicalDeviceProperties props;
    for (uint32_t i = 0; i < n; i++) {
        vkGetPhysicalDeviceProperties(devs[i], &props);
        printf("vk-clear: device %u: %s (API %u.%u)\n", i, props.deviceName, VK_API_VERSION_MAJOR(props.apiVersion),
               VK_API_VERSION_MINOR(props.apiVersion));
        if (pd == VK_NULL_HANDLE && strstr(props.deviceName, want))
            pd = devs[i];
    }
    if (pd == VK_NULL_HANDLE) {
        printf("vk-clear: FAIL no device named like \"%s\"\n", want);
        return 1;
    }
    vkGetPhysicalDeviceProperties(pd, &props);
    uint32_t nq = 0;
    vkGetPhysicalDeviceQueueFamilyProperties(pd, &nq, NULL);
    VkQueueFamilyProperties qf[16];
    if (nq > 16)
        nq = 16;
    vkGetPhysicalDeviceQueueFamilyProperties(pd, &nq, qf);
    uint32_t family = UINT32_MAX;
    for (uint32_t i = 0; i < nq; i++)
        if (qf[i].queueFlags & (VK_QUEUE_GRAPHICS_BIT | VK_QUEUE_COMPUTE_BIT | VK_QUEUE_TRANSFER_BIT)) {
            family = i;
            break;
        }
    if (family == UINT32_MAX) {
        printf("vk-clear: FAIL no queue family\n");
        return 1;
    }
    float prio = 1.0f;
    VkDeviceQueueCreateInfo qci = {.sType = VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO,
                                   .queueFamilyIndex = family,
                                   .queueCount = 1,
                                   .pQueuePriorities = &prio};
    VkDeviceCreateInfo dci = {.sType = VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO, .queueCreateInfoCount = 1, .pQueueCreateInfos = &qci};
    VkDevice dev;
    CHECK(vkCreateDevice(pd, &dci, NULL, &dev));
    VkQueue queue;
    vkGetDeviceQueue(dev, family, 0, &queue);
    VkPhysicalDeviceMemoryProperties mp;
    vkGetPhysicalDeviceMemoryProperties(pd, &mp);

    /* The image, cleared on the GPU. */
    VkImageCreateInfo imci = {.sType = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO,
                              .imageType = VK_IMAGE_TYPE_2D,
                              .format = VK_FORMAT_R8G8B8A8_UNORM,
                              .extent = {SIDE, SIDE, 1},
                              .mipLevels = 1,
                              .arrayLayers = 1,
                              .samples = VK_SAMPLE_COUNT_1_BIT,
                              .tiling = VK_IMAGE_TILING_OPTIMAL,
                              .usage = VK_IMAGE_USAGE_TRANSFER_DST_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT,
                              .initialLayout = VK_IMAGE_LAYOUT_UNDEFINED};
    VkImage image;
    CHECK(vkCreateImage(dev, &imci, NULL, &image));
    VkMemoryRequirements req;
    vkGetImageMemoryRequirements(dev, image, &req);
    int it = memory_type(&mp, req.memoryTypeBits, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);
    if (it < 0)
        it = memory_type(&mp, req.memoryTypeBits, 0);
    VkMemoryAllocateInfo mai = {.sType = VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO, .allocationSize = req.size, .memoryTypeIndex = (uint32_t)it};
    VkDeviceMemory image_mem;
    CHECK(vkAllocateMemory(dev, &mai, NULL, &image_mem));
    CHECK(vkBindImageMemory(dev, image, image_mem, 0));

    /* The buffer it is copied to, which the CPU reads. */
    VkBufferCreateInfo bci = {.sType = VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO,
                              .size = SIDE * SIDE * 4,
                              .usage = VK_BUFFER_USAGE_TRANSFER_DST_BIT};
    VkBuffer buffer;
    CHECK(vkCreateBuffer(dev, &bci, NULL, &buffer));
    vkGetBufferMemoryRequirements(dev, buffer, &req);
    int bt = memory_type(&mp, req.memoryTypeBits, VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT | VK_MEMORY_PROPERTY_HOST_COHERENT_BIT);
    if (bt < 0) {
        printf("vk-clear: FAIL no host-visible coherent memory\n");
        return 1;
    }
    mai.allocationSize = req.size;
    mai.memoryTypeIndex = (uint32_t)bt;
    VkDeviceMemory buffer_mem;
    CHECK(vkAllocateMemory(dev, &mai, NULL, &buffer_mem));
    CHECK(vkBindBufferMemory(dev, buffer, buffer_mem, 0));
    unsigned char *mapped;
    CHECK(vkMapMemory(dev, buffer_mem, 0, VK_WHOLE_SIZE, 0, (void **)&mapped));
    memset(mapped, 0x5a, SIDE * SIDE * 4);

    VkCommandPoolCreateInfo pci = {.sType = VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO, .queueFamilyIndex = family};
    VkCommandPool pool;
    CHECK(vkCreateCommandPool(dev, &pci, NULL, &pool));
    VkCommandBufferAllocateInfo cai = {.sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
                                       .commandPool = pool,
                                       .level = VK_COMMAND_BUFFER_LEVEL_PRIMARY,
                                       .commandBufferCount = 1};
    VkCommandBuffer cmd;
    CHECK(vkAllocateCommandBuffers(dev, &cai, &cmd));
    VkCommandBufferBeginInfo begin = {.sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO,
                                      .flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT};
    CHECK(vkBeginCommandBuffer(cmd, &begin));
    VkImageSubresourceRange range = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
    VkImageMemoryBarrier to_dst = {.sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
                                   .dstAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT,
                                   .oldLayout = VK_IMAGE_LAYOUT_UNDEFINED,
                                   .newLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL,
                                   .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
                                   .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
                                   .image = image,
                                   .subresourceRange = range};
    vkCmdPipelineBarrier(cmd, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 0, NULL, 0, NULL, 1, &to_dst);
    VkClearColorValue colour = {.float32 = {0.25f, 0.5f, 0.75f, 1.0f}};
    vkCmdClearColorImage(cmd, image, VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL, &colour, 1, &range);
    VkImageMemoryBarrier to_src = to_dst;
    to_src.srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT;
    to_src.dstAccessMask = VK_ACCESS_TRANSFER_READ_BIT;
    to_src.oldLayout = VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
    to_src.newLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
    vkCmdPipelineBarrier(cmd, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 0, NULL, 0, NULL, 1, &to_src);
    VkBufferImageCopy copy = {.imageSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, 1}, .imageExtent = {SIDE, SIDE, 1}};
    vkCmdCopyImageToBuffer(cmd, image, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, buffer, 1, &copy);
    VkBufferMemoryBarrier to_host = {.sType = VK_STRUCTURE_TYPE_BUFFER_MEMORY_BARRIER,
                                     .srcAccessMask = VK_ACCESS_TRANSFER_WRITE_BIT,
                                     .dstAccessMask = VK_ACCESS_HOST_READ_BIT,
                                     .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
                                     .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
                                     .buffer = buffer,
                                     .size = VK_WHOLE_SIZE};
    vkCmdPipelineBarrier(cmd, VK_PIPELINE_STAGE_TRANSFER_BIT, VK_PIPELINE_STAGE_HOST_BIT, 0, 0, NULL, 1, &to_host, 0, NULL);
    CHECK(vkEndCommandBuffer(cmd));
    VkFenceCreateInfo fci = {.sType = VK_STRUCTURE_TYPE_FENCE_CREATE_INFO};
    VkFence fence;
    CHECK(vkCreateFence(dev, &fci, NULL, &fence));
    VkSubmitInfo si = {.sType = VK_STRUCTURE_TYPE_SUBMIT_INFO, .commandBufferCount = 1, .pCommandBuffers = &cmd};
    CHECK(vkQueueSubmit(queue, 1, &si, fence));
    CHECK(vkWaitForFences(dev, 1, &fence, VK_TRUE, 30ull * 1000 * 1000 * 1000));

    int bad = 0;
    for (int i = 0; i < SIDE * SIDE; i++) {
        const unsigned char *p = mapped + i * 4;
        /* 0.25, 0.5, 0.75, 1.0 in UNORM8: 64, 128, 191, 255 (+-1 for rounding). */
        if (abs(p[0] - 64) > 1 || abs(p[1] - 128) > 1 || abs(p[2] - 191) > 1 || p[3] != 255)
            bad++;
    }
    printf("vk-clear: %s: pixel 0 = %u %u %u %u, %d of %d wrong\n", props.deviceName, mapped[0], mapped[1], mapped[2],
           mapped[3], bad, SIDE * SIDE);
    vkUnmapMemory(dev, buffer_mem);
    vkDestroyFence(dev, fence, NULL);
    vkDestroyCommandPool(dev, pool, NULL);
    vkDestroyBuffer(dev, buffer, NULL);
    vkFreeMemory(dev, buffer_mem, NULL);
    vkDestroyImage(dev, image, NULL);
    vkFreeMemory(dev, image_mem, NULL);
    vkDestroyDevice(dev, NULL);
    vkDestroyInstance(inst, NULL);
    if (bad) {
        printf("vk-clear: FAIL\n");
        return 1;
    }
    printf("vk-clear: ok\n");
    return 0;
}
